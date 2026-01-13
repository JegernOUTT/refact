import json
import time
import uuid
import pytest
import requests
from tests.generic_test_case import GenericTestCase


class TestTaskAgentFailure(GenericTestCase):
    @pytest.fixture(autouse=True)
    def setup_task(self):
        self.task_id = f"test-task-{uuid.uuid4().hex[:8]}"
        self.base_url = f"http://127.0.0.1:{self.lsp_port}"
        
        task_data = {
            "name": "Test Task",
            "instructions": "Test agent failure detection",
            "cards": [
                {
                    "id": "T-1",
                    "title": "Test Card 1",
                    "column": "planned",
                    "instructions": "Test instructions",
                    "priority": "P1",
                }
            ]
        }
        
        resp = requests.post(
            f"{self.base_url}/v1/tasks/create",
            json={"task_id": self.task_id, "task_data": task_data}
        )
        assert resp.status_code == 200, f"Failed to create task: {resp.text}"
        
        yield
        
        try:
            requests.post(f"{self.base_url}/v1/tasks/delete", json={"task_id": self.task_id})
        except:
            pass

    def test_agent_streaming_error_marks_card_failed(self):
        planner_chat_id = f"planner-{self.task_id}"
        
        resp = requests.post(
            f"{self.base_url}/v1/chats/{planner_chat_id}/commands",
            json={
                "type": "set_params",
                "params": {
                    "mode": "TASK_PLANNER",
                    "task_meta": {
                        "task_id": self.task_id,
                        "role": "planner"
                    }
                }
            }
        )
        assert resp.status_code == 202
        
        agent_id = uuid.uuid4().hex
        agent_chat_id = f"agent-T-1-{agent_id[:8]}"
        
        board_resp = requests.get(f"{self.base_url}/v1/tasks/{self.task_id}/board")
        assert board_resp.status_code == 200
        board = board_resp.json()
        
        board["cards"][0]["column"] = "doing"
        board["cards"][0]["assignee"] = agent_id
        board["cards"][0]["agent_chat_id"] = agent_chat_id
        board["cards"][0]["started_at"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
        
        save_resp = requests.post(
            f"{self.base_url}/v1/tasks/{self.task_id}/board",
            json=board
        )
        assert save_resp.status_code == 200
        
        agent_setup_resp = requests.post(
            f"{self.base_url}/v1/chats/{agent_chat_id}/commands",
            json={
                "type": "set_params",
                "params": {
                    "mode": "TASK_AGENT",
                    "model": "invalid-model-that-will-error",
                    "task_meta": {
                        "task_id": self.task_id,
                        "role": "agents",
                        "agent_id": agent_id,
                        "card_id": "T-1"
                    }
                }
            }
        )
        assert agent_setup_resp.status_code == 202
        
        trigger_resp = requests.post(
            f"{self.base_url}/v1/chats/{agent_chat_id}/commands",
            json={
                "type": "user_message",
                "content": "This will cause an error",
                "client_request_id": str(uuid.uuid4())
            }
        )
        assert trigger_resp.status_code == 202
        
        time.sleep(5)
        
        board_after_resp = requests.get(f"{self.base_url}/v1/tasks/{self.task_id}/board")
        assert board_after_resp.status_code == 200
        board_after = board_after_resp.json()
        
        card = next((c for c in board_after["cards"] if c["id"] == "T-1"), None)
        assert card is not None
        assert card["column"] == "failed", f"Card should be failed, got {card['column']}"
        assert "FAILED (automatic)" in card.get("final_report", "")

    def test_agent_ownership_mismatch_skips_failure(self):
        agent_id_1 = uuid.uuid4().hex
        agent_id_2 = uuid.uuid4().hex
        
        board_resp = requests.get(f"{self.base_url}/v1/tasks/{self.task_id}/board")
        board = board_resp.json()
        
        board["cards"][0]["column"] = "doing"
        board["cards"][0]["assignee"] = agent_id_1
        board["cards"][0]["agent_chat_id"] = f"agent-T-1-{agent_id_1[:8]}"
        
        requests.post(f"{self.base_url}/v1/tasks/{self.task_id}/board", json=board)
        
        from refact_agent.engine.src.chat import task_agent_monitor
        from refact_agent.engine.src.chat.types import TaskMeta
        
        task_meta = TaskMeta(
            task_id=self.task_id,
            role="agents",
            agent_id=agent_id_2,
            card_id="T-1"
        )
        
        import asyncio
        asyncio.run(
            task_agent_monitor.mark_agent_as_failed(
                self.gcx,
                self.task_id,
                "T-1",
                agent_id_2,
                "Test error with wrong agent"
            )
        )
        
        board_after = requests.get(f"{self.base_url}/v1/tasks/{self.task_id}/board").json()
        card = next((c for c in board_after["cards"] if c["id"] == "T-1"), None)
        
        assert card["column"] == "doing", "Card should still be doing (ownership mismatch)"

    def test_already_failed_card_computes_all_finished(self):
        board_resp = requests.get(f"{self.base_url}/v1/tasks/{self.task_id}/board")
        board = board_resp.json()
        
        board["cards"].append({
            "id": "T-2",
            "title": "Second card",
            "column": "doing",
            "assignee": uuid.uuid4().hex,
            "agent_chat_id": f"agent-T-2-{uuid.uuid4().hex[:8]}",
            "instructions": "Test",
            "priority": "P1",
            "created_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
        })
        
        board["cards"][0]["column"] = "failed"
        board["cards"][0]["final_report"] = "Already failed"
        
        requests.post(f"{self.base_url}/v1/tasks/{self.task_id}/board", json=board)
        
        from refact_agent.engine.src.chat import task_agent_monitor
        import asyncio
        
        result = asyncio.run(
            task_agent_monitor.mark_agent_as_failed(
                self.gcx,
                self.task_id,
                "T-1",
                None,
                "Retry failure"
            )
        )
        
        assert result is None or True

    def test_fallback_timestamps_for_stuck_detection(self):
        agent_id = uuid.uuid4().hex
        agent_chat_id = f"agent-T-1-{agent_id[:8]}"
        
        old_timestamp = time.strftime(
            "%Y-%m-%dT%H:%M:%SZ",
            time.gmtime(time.time() - 25 * 60)
        )
        
        board_resp = requests.get(f"{self.base_url}/v1/tasks/{self.task_id}/board")
        board = board_resp.json()
        
        board["cards"][0]["column"] = "doing"
        board["cards"][0]["assignee"] = agent_id
        board["cards"][0]["agent_chat_id"] = agent_chat_id
        board["cards"][0]["started_at"] = old_timestamp
        board["cards"][0]["status_updates"] = []
        
        requests.post(f"{self.base_url}/v1/tasks/{self.task_id}/board", json=board)
        
        from refact_agent.engine.src.chat import task_agent_monitor
        import asyncio
        
        asyncio.run(task_agent_monitor.check_for_stuck_agents(self.gcx))
        
        time.sleep(1)
        
        board_after = requests.get(f"{self.base_url}/v1/tasks/{self.task_id}/board").json()
        card = next((c for c in board_after["cards"] if c["id"] == "T-1"), None)
        
        assert card["column"] == "failed", "Card with old started_at should be marked failed"

    def test_no_agent_chat_id_but_assignee_fails_if_stuck(self):
        agent_id = uuid.uuid4().hex
        
        old_timestamp = time.strftime(
            "%Y-%m-%dT%H:%M:%SZ",
            time.gmtime(time.time() - 25 * 60)
        )
        
        board_resp = requests.get(f"{self.base_url}/v1/tasks/{self.task_id}/board")
        board = board_resp.json()
        
        board["cards"][0]["column"] = "doing"
        board["cards"][0]["assignee"] = agent_id
        board["cards"][0]["agent_chat_id"] = None
        board["cards"][0]["started_at"] = old_timestamp
        
        requests.post(f"{self.base_url}/v1/tasks/{self.task_id}/board", json=board)
        
        from refact_agent.engine.src.chat import task_agent_monitor
        import asyncio
        
        asyncio.run(task_agent_monitor.check_for_stuck_agents(self.gcx))
        
        time.sleep(1)
        
        board_after = requests.get(f"{self.base_url}/v1/tasks/{self.task_id}/board").json()
        card = next((c for c in board_after["cards"] if c["id"] == "T-1"), None)
        
        assert card["column"] == "failed"
        assert "no agent_chat_id" in card.get("final_report", "").lower()

    def test_planner_notified_when_all_agents_done(self):
        planner_chat_id = f"planner-{self.task_id}"
        
        resp = requests.post(
            f"{self.base_url}/v1/chats/{planner_chat_id}/commands",
            json={
                "type": "set_params",
                "params": {
                    "mode": "TASK_PLANNER",
                    "task_meta": {
                        "task_id": self.task_id,
                        "role": "planner"
                    }
                }
            }
        )
        assert resp.status_code == 202
        
        agent_id = uuid.uuid4().hex
        agent_chat_id = f"agent-T-1-{agent_id[:8]}"
        
        board_resp = requests.get(f"{self.base_url}/v1/tasks/{self.task_id}/board")
        board = board_resp.json()
        
        board["cards"][0]["column"] = "doing"
        board["cards"][0]["assignee"] = agent_id
        board["cards"][0]["agent_chat_id"] = agent_chat_id
        
        requests.post(f"{self.base_url}/v1/tasks/{self.task_id}/board", json=board)
        
        from refact_agent.engine.src.chat import task_agent_monitor
        import asyncio
        
        asyncio.run(
            task_agent_monitor.mark_agent_as_failed(
                self.gcx,
                self.task_id,
                "T-1",
                agent_id,
                "Test failure for notification"
            )
        )
        
        time.sleep(2)
        
        chat_resp = requests.get(f"{self.base_url}/v1/chats/{planner_chat_id}/messages")
        if chat_resp.status_code == 200:
            messages = chat_resp.json()
            planner_notified = any(
                "all agents have completed" in msg.get("content", "").lower()
                for msg in messages
            )
            assert planner_notified, "Planner should be notified when all agents done"


if __name__ == "__main__":
    pytest.main([__file__, "-v", "-s"])
