package com.smallcloud.refactai.notifications

import com.google.gson.JsonParser
import com.smallcloud.refactai.panes.sharedchat.Events
import org.junit.Assert.*
import org.junit.Test

class DecisionRegistryTest {
    @Test fun endpointIdentityMatchesGuiEngineRootNormalization() {
        assertEquals("http://localhost/p/project", decisionEndpointIdentity(" HTTP://LOCALHOST:80/p/project/v1/chat?x=1#frag "))
        assertEquals("https://example.org/p/project", decisionEndpointIdentity("https://EXAMPLE.ORG:443/p/project/v1/"))
        assertEquals("http://localhost:8001/p/v1/private", decisionEndpointIdentity("http://localhost:8001/p/v1/private/"))
        assertEquals("http://localhost:8001/p/project", decisionEndpointIdentity("http://localhost:8001/p/project/v1/code-intel/health"))
        assertNull(decisionEndpointIdentity("file:///tmp/project"))
    }
    private val scope = DecisionScope("/project", "http://localhost:8001/p/project")
    private fun tracker() = DecisionRegistry().apply { bind(scope) }
    private fun ask(time: Long? = 10, tool: String = "t") = Events.AskQuestionsPayload(
        "c", tool, listOf(Events.QuestionItem("q", "yes_no", "Proceed?")), time)
    private fun resolve(time: Long? = 10, tool: String = "t") = Events.DecisionResolvedPayload("c", tool, "answered", time)

    @Test fun goldenDigestCorpusMatchesTypeScript() {
        val cases = listOf(
            Events.QuestionItem("proceed", "yes_no", "Proceed with scratch check?") to "beee8ea4",
            Events.QuestionItem("pond", "single_select", "Pick a pond", listOf("north", "south")) to "539fd1f6",
            Events.QuestionItem("pond", "single_select", "Pick a pond", listOf("south", "north")) to "12c5c376",
            Events.QuestionItem("note", "free_text", "Отчёт: продолжить? ✅") to "26382507"
        )
        for ((question, expected) in cases) assertEquals(expected, DecisionRegistry.digest(listOf(question)))
    }

    @Test fun guiAndSseShareCorrelationAndOpenDoesNotResolve() {
        val tracker = tracker()
        assertTrue(tracker.announce(ask().copy(origin = "native_chat_event"), scope).announce)
        assertEquals("duplicate", tracker.announce(ask().copy(origin = "native_notification_stream"), scope).reason)
        assertTrue(tracker.contains(DecisionRegistry.key("c", "t"))) // what Open Chat reads; no mutation
        assertEquals(1, tracker.count())
        assertTrue(tracker.resolve(resolve(), scope))
        assertEquals("already_resolved", tracker.announce(ask(), scope).reason)
        assertEquals(0, tracker.count())
    }

    @Test fun staleIncarnationsCannotReplaceOrResolveNewerAsk() {
        val tracker = tracker()
        tracker.announce(ask(20), scope)
        assertEquals("duplicate", tracker.announce(ask(10), scope).reason)
        assertEquals("duplicate", tracker.announce(ask(null), scope).reason)
        assertFalse(tracker.resolve(resolve(10), scope))
        assertFalse(tracker.resolve(resolve(null), scope))
        assertTrue(tracker.resolve(resolve(20), scope))
        assertFalse(tracker.announce(ask(10), scope).announce)
        assertTrue(tracker.announce(ask(21), scope).announce)
    }

    @Test fun supersededContentIsNotAnotherNotification() {
        val tracker = tracker()
        tracker.announce(ask(), scope)
        assertEquals("superseded_content", tracker.announce(ask().copy(
            questions = listOf(Events.QuestionItem("q", "free_text", "Updated"))), scope).reason)
        assertEquals(1, tracker.count())
    }

    @Test fun unknownResolutionTombstonesReplayAndDoesNotRegress() {
        val tracker = tracker()
        assertFalse(tracker.resolve(resolve(20), scope))
        tracker.resolve(resolve(10), scope)
        assertFalse(tracker.announce(ask(15), scope).announce)
        assertTrue(tracker.announce(ask(21), scope).announce)
        val legacy = this.tracker()
        legacy.resolve(resolve(null), scope)
        assertEquals(0, legacy.tombstoneCount())
        assertTrue(legacy.announce(ask(null), scope).announce)
        legacy.resolve(resolve(null), scope)
        assertFalse(legacy.announce(ask(30), scope).announce)
    }

    @Test fun endpointAndProjectIsolationAndSameEndpointReconnect() {
        val tracker = tracker()
        tracker.announce(ask(), scope)
        tracker.bind(scope)
        assertFalse(tracker.announce(ask(), scope).announce)
        val other = scope.copy(endpointIdentity = "http://localhost:8001/p/other")
        assertEquals("foreign_scope", tracker.announce(ask(), other).reason)
        assertFalse(tracker.resolve(resolve(), other))
        assertFalse(tracker.announce(ask().copy(scope = other), scope).announce)
        assertFalse(tracker.resolve(resolve().copy(scope = scope.copy(projectRoot = "/other")), scope))
        tracker.bind(other)
        assertEquals(0, tracker.count())
        assertFalse(tracker.announce(ask(), scope).announce)
        assertTrue(tracker.announce(ask(), other).announce)
    }

    @Test fun boundedOutstandingAndTombstones() {
        val tracker = tracker()
        repeat(250) { tracker.announce(ask(tool = "$it"), scope) }
        assertEquals(200, tracker.count())
        repeat(250) { tracker.resolve(resolve(tool = "$it"), scope) }
        assertEquals(200, tracker.tombstoneCount())
        assertEquals(0, tracker.count())
        assertFalse(tracker.announce(ask(tool = "249"), scope).announce)
    }

    @Test fun tolerantWireParsingRejectsMalformedAndUnknownOutcomes() {
        val valid = """{"chatId":"c","toolCallId":"t","questions":[{"id":"q","type":"future_question_type","text":"","extra":true}],"extra":1}"""
        assertNotNull(DecisionWire.ask(JsonParser.parseString(valid)))
        for (bad in listOf("null", "[]", "{}", valid.replace("\"c\"", "1"),
            valid.replace("\"extra\":1", "\"askedAtMs\":1.5"),
            valid.replace("\"extra\":1", "\"askedAtMs\":9007199254740992"),
            valid.replace("\"extra\":1", "\"origin\":\"host_approved\""),
            valid.replace("\"extra\":true", "\"options\":[1]"))) {
            assertNull(bad, DecisionWire.ask(JsonParser.parseString(bad)))
        }
        assertNull(DecisionWire.resolved(JsonParser.parseString("""{"chatId":"c","toolCallId":"t","resolution":"approved"}""")))
        for (outcome in listOf("answered", "cancelled", "resolved_elsewhere")) {
            assertNotNull(Events.parse("""{"type":"ide/decisionResolved","payload":{"chatId":"c","toolCallId":"t","resolution":"$outcome","askedAtMs":10}}"""))
        }
    }

    @Test fun nativeWireRequiresCorrelationAndPreservesIncarnation() {
        val native = JsonParser.parseString("""{"chat_id":"c","tool_call_id":"t","asked_at_ms":42,"questions":[{"id":"q","type":"yes_no","text":"Proceed?"}]}""").asJsonObject
        val payload = DecisionWire.ask(DecisionWire.nativePayload(native))!!
        assertEquals(42L, payload.askedAtMs)
        assertEquals("t", payload.toolCallId)
        native.remove("tool_call_id")
        assertNull(DecisionWire.ask(DecisionWire.nativePayload(native)))
    }
}
