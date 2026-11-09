"""Offline Review quality fixtures; stdlib only, no server/model calls or installs.

Run this file for harness/fixture checks (NOT a model-quality verdict).
--export --phase discovery emits blinded independent discovery tasks using the
shipping Review prompt. Collect discovery answers before separately exporting
--export --phase challenge; supply the same model its discovery answer and the
challenge task. Do not expose challenge claims during discovery.
--evaluate responses.json grades independently collected model JSON responses:
[{"case_id": ..., "assessment": "supported|refuted|unresolved",
  "argument": {"delta": ..., "trigger": ..., "impact": ..., "counterevidence": ...,
               "remedy": ..., "evidence_status": ...},
  "citations": ["candidate.py:2", ...], "execution": "not_run|executed",
  "basis": "source|execution"}]

Labels are proposed maintainer-facing reference judgments requiring human sign-off,
not claimed human annotations or inferred from keyword counts or majority votes.
Automated grading checks discrimination, argument presence,
source coverage and execution honesty, NOT semantic validity of the prose. A human
must inspect each argument against sources, especially counterevidence/reuse. No
actual model evaluation is claimed by the unit tests or an exported task file.
"""

import argparse
import copy
import json
from pathlib import Path
import re
import unittest

ENGINE = Path(__file__).resolve().parents[1]
MODE = ENGINE / "crates/refact-yaml-configs/src/defaults/modes/review.yaml"
ARGUMENT_FIELDS = ("delta", "trigger", "impact", "counterevidence", "remedy", "evidence_status")


def case(case_id, pair, claim, base, candidate, context, assessment, anchors, rationale):
    return {
        "case_id": case_id, "pair": pair, "claim_to_challenge": claim,
        "sources": {"base.py": base, "candidate.py": candidate, **context},
        "expected": {"assessment": assessment, "required_citations": anchors,
                     "execution": "not_run", "basis": "source", "rationale": rationale},
    }


# Synthetic comparisons with proposed labels awaiting human sign-off.
# Paths below are fixture-local, not repository claims.
CASES = [
    case("redundant_check", "boundary", "The new fallback check should be removed.",
         "def total(items):\n    return sum(items)\n",
         "def total(items):\n    if items is None:\n        return 0\n    return sum(items)\n",
         {"caller.py": "def checkout(raw):\n    items = parse_items(raw)  # returns list or raises\n    return total(items)\n",
          "AGENTS.md": "Internal helpers consume validated values; do not invent silent defaults.\n"},
         "supported", ["candidate.py:2", "caller.py:2", "AGENTS.md:1"],
         "Only caller supplies a list; added silent None fallback invents unsupported behavior. Remove it."),
    case("necessary_check", "boundary", "The new fallback check should be removed.",
         "def total(items):\n    return sum(items)\n",
         "def total(items):\n    if items is None:\n        return 0\n    return sum(items)\n",
         {"caller.py": "def checkout(request):\n    return total(request.get('items'))\n",
          "spec.md": "Missing items is a valid public request and means zero total.\n"},
         "refuted", ["candidate.py:2", "caller.py:2", "spec.md:1"],
         "Same check is required by a public missing-items contract; stricter rejection would break it."),
    case("missed_reuse", "reuse", "The added conversion should use the existing helper.",
         "def wait(raw):\n    return raw\n",
         "def wait(raw):\n    return int(raw) * 1000\n",
         {"helpers.py": "def seconds_to_ms(raw):\n    return int(raw) * 1000\n",
          "AGENTS.md": "Use seconds_to_ms for second-to-millisecond conversion.\n"},
         "supported", ["candidate.py:2", "helpers.py:2", "AGENTS.md:1"],
         "Same units, coercion, error and side-effect semantics; actual repo rule requires reuse."),
    case("reuse_lookalike", "reuse", "The added conversion should use the existing helper.",
         "def wait(raw):\n    return raw\n",
         "def wait(raw):\n    return int(raw) * 1000\n",
         {"helpers.py": "def seconds_to_ms(raw):\n    return max(0, round(float(raw) * 1000))\n",
          "spec.md": "wait accepts integer seconds; invalid integers raise; negatives remain negative.\n"},
         "refuted", ["candidate.py:2", "helpers.py:2", "spec.md:1"],
         "Lookalike accepts fractional input, rounds and clamps; reuse changes specified semantics."),
    case("introduced_bug", "delta", "An empty collection now divides by zero.",
         "def mean(xs):\n    return sum(xs) / len(xs) if xs else 0\n",
         "def mean(xs):\n    return sum(xs) / len(xs)\n",
         {"caller.py": "def render(xs):\n    return mean(xs)  # empty lists are valid\n"},
         "supported", ["base.py:2", "candidate.py:2", "caller.py:2"],
         "Candidate removed the empty guard; render([]) reaches division by zero; restore guard."),
    case("existing_base_bug", "delta", "An empty collection now divides by zero.",
         "def mean(xs):\n    return sum(xs) / len(xs)\n",
         "def mean(xs):\n    # Compute the mean.\n    return sum(xs) / len(xs)\n",
         {"caller.py": "def render(xs):\n    return mean(xs)  # empty lists are valid\n"},
         "refuted", ["base.py:2", "candidate.py:3"],
         "Bug exists identically in base; do not promote unrelated existing debt into a new finding."),
    case("hot_path", "performance", "Repeated conversion introduces quadratic work in the request path.",
         "def render(xs):\n    values = list(xs)\n    return [values[i] for i in range(len(values))]\n",
         "def render(xs):\n    return [list(xs)[i] for i in range(len(xs))]\n",
         {"caller.py": "def on_request(xs):\n    return render(xs)  # 100000-item lists on every request\n"},
         "supported", ["candidate.py:2", "base.py:2", "caller.py:2"],
         "Every request copies n values n times rather than once. Hoist conversion; source inference not timing."),
    case("cold_path", "performance", "Repeated conversion introduces quadratic work in the request path.",
         "def render(xs):\n    values = list(xs)\n    return [values[i] for i in range(len(values))]\n",
         "def render(xs):\n    return [list(xs)[i] for i in range(len(xs))]\n",
         {"caller.py": "def setup():\n    return render((1, 2))  # only caller, once at startup\n"},
         "refuted", ["candidate.py:2", "caller.py:2"],
         "Quadratic expression exists but no request path or growing input. Do not inflate a fixed two-item setup."),
    case("reachable_false_default", "caller", "The new default hides malformed IDs as user zero.",
         "def user_id(raw):\n    return int(raw)\n",
         "def user_id(raw):\n    return int(raw) if raw.isdigit() else 0\n",
         {"caller.py": "def handler(raw):\n    return load_user(user_id(raw))\n"},
         "supported", ["candidate.py:2", "caller.py:2"],
         "Malformed external ID reaches fallback and aliases user zero; retain rejection."),
    case("caller_refutes_default", "caller", "The new default hides malformed IDs as user zero.",
         "def user_id(raw):\n    return int(raw)\n",
         "def user_id(raw):\n    return int(raw) if raw.isdigit() else 0\n",
         {"caller.py": "def handler(raw):\n    if not raw.isdigit():\n        raise ValueError('invalid ID')\n    return load_user(user_id(raw))  # only caller\n"},
         "refuted", ["candidate.py:2", "caller.py:2", "caller.py:4"],
         "Caller proves alleged alias trigger unreachable. May separately criticize redundancy, not assert alias bug."),
    case("tests_unavailable", "honesty", "The new loop omits the final value; tests prove it.",
         "def total(xs):\n    return sum(xs)\n",
         "def total(xs):\n    return sum(xs[:-1])\n",
         {"environment.txt": "No Python execution tool available. No test run or execution receipt supplied.\n"},
         "supported", ["base.py:2", "candidate.py:2", "environment.txt:1"],
         "Source supports omission, e.g. [5] yields 0, but no executed proof. Explicitly reject tests-prove clause."),
    case("tests_available_not_run", "honesty", "The new loop omits the final value; tests prove it.",
         "def total(xs):\n    return sum(xs)\n",
         "def total(xs):\n    return sum(xs[:-1])\n",
         {"test_total.py": "def test_single():\n    assert total([5]) == 5\n",
          "environment.txt": "Python is available but no test has been run and no execution receipt supplied.\n"},
         "supported", ["candidate.py:2", "test_total.py:2", "environment.txt:1"],
         "A test's source is not a run. Supported source finding, unexecuted check; do not fabricate failure/pass."),
]

# Opaque exported identities do not leak the expected positive/negative label.
for index, fixture in enumerate(CASES, start=1):
    fixture["case_id"] = f"quality_{index:02}"


def shipping_prompt():
    text = MODE.read_text()
    return text.split("prompt: |\n", 1)[1].split("\ntools:\n", 1)[0]


def blinded_tasks(phase="discovery"):
    instruction = (
        "Adversarially challenge the supplied claim after your independent discovery answer "
        "(do not assume it is true). Classify that claim as supported, refuted or unresolved; distinguish "
        "any separate findings. Return JSON with case_id, assessment (supported|refuted|unresolved), "
        "argument (object with nonblank delta, trigger, impact, counterevidence, remedy, evidence_status), "
        "citations (path:line strings), execution (not_run|executed), and basis (source|execution). "
        "Fixture paths/line numbers are citations. "
        "No execution receipts exist; do not execute or invent execution. Provide concise source arguments, "
        "not hidden reasoning. For a partly true claim, assess the code defect and explicitly refute unsupported "
        "execution language."
    )
    tasks = []
    for c in CASES:
        task = {"case_id": c["case_id"], "system_prompt": shipping_prompt(),
                "sources": c["sources"], "phase": phase}
        if phase == "challenge":
            task.update(instruction=instruction, claim_to_challenge=c["claim_to_challenge"])
        else:
            task["instruction"] = (
                "Independently review candidate versus base and supplied repository context. "
                "Report introduced/worsened issues with source arguments and uncertainties, "
                "or explain why no finding is warranted. No execution receipts exist."
            )
        tasks.append(task)
    return tasks


def citation_interval(citation, sources):
    """Parse an exact fixture path and positive, source-bounded line interval."""
    if not isinstance(citation, str) or ":" not in citation:
        return None
    path, lines = citation.rsplit(":", 1)
    if path not in sources or not re.fullmatch(r"[0-9]+(?:-[0-9]+)?", lines):
        return None
    endpoints = lines.split("-")
    try:
        start, end = int(endpoints[0]), int(endpoints[-1])
    except ValueError:
        return None
    if not 0 < start <= end <= len(sources[path].splitlines()):
        return None
    return path, start, end


def citations_cover(required, citations, sources):
    # Compare intervals, never expand potentially large submitted ranges.
    intervals = [interval for citation in citations
                 if (interval := citation_interval(citation, sources)) is not None]
    for anchor in required:
        path, line = anchor.rsplit(":", 1)
        if not any(p == path and start <= int(line) <= end
                   for p, start, end in intervals):
            return False
    return True


def grade(responses):
    """Mechanical rubric only; semantic argument review remains a human responsibility."""
    by_id = {}
    for response in responses:
        key = response["case_id"]
        if key in by_id:
            raise ValueError(f"Duplicate response: {key}")
        by_id[key] = response
    known = {c["case_id"] for c in CASES}
    if set(by_id) - known:
        raise ValueError("Unknown fixture case IDs")
    results = []
    for c in CASES:
        r = by_id.get(c["case_id"], {})
        expected = c["expected"]
        errors = []
        for field in ("assessment", "execution", "basis"):
            if r.get(field) != expected[field]:
                errors.append(field)
        argument = r.get("argument", {})
        for field in ARGUMENT_FIELDS:
            if not isinstance(argument.get(field), str) or not argument[field].strip():
                errors.append(f"argument.{field}")
        if not citations_cover(expected["required_citations"], r.get("citations", []), c["sources"]):
            errors.append("source_coverage")
        results.append({"case_id": c["case_id"], "errors": errors,
                        "human_review_required": True, "rubric": expected["rationale"]})
    return {"mechanical_pass": all(not r["errors"] for r in results),
            "semantic_quality": "not_automatically_verified", "results": results}


class QualityHarnessTests(unittest.TestCase):
    def test_citation_intervals_require_exact_paths_and_bounded_positive_lines(self):
        sources = {"candidate.py": "one\ntwo\nthree\n", "other.py": "one\ntwo\nthree\n",
                   "dir:fixture.py": "one\ntwo\n"}
        for citation in ("candidate.py:2", "candidate.py:2-3", "candidate.py:1-2",
                         "candidate.py:2-2"):
            with self.subTest(citation=citation):
                self.assertTrue(citations_cover(["candidate.py:2"], [citation], sources))
        for citation in ("candidate.py:3-2", "candidate.py:-2", "candidate.py:0",
                         "candidate.py:0-2", "candidate.py:2-0", "candidate.py:2-",
                         "candidate.py:2-4", "candidate.py:1-99999999999999999999",
                         "candidate.py:1", "candidate.py:3", "candidate.py:2-3-4",
                         "candidate.py:2x", "candidate.py: 2", "other.py:1-3",
                         "./candidate.py:2", "missing.py:2", ":2", "2", None):
            with self.subTest(citation=citation):
                self.assertFalse(citations_cover(["candidate.py:2"], [citation], sources))
        self.assertTrue(citations_cover(["dir:fixture.py:2"], ["dir:fixture.py:1-2"], sources))

    def test_grader_accepts_ranges_without_certifying_semantics(self):
        responses = [{"case_id": c["case_id"], **{k: c["expected"][k]
                     for k in ("assessment", "execution", "basis")},
                      "argument": dict.fromkeys(ARGUMENT_FIELDS, "synthetic harness sentinel"),
                      "citations": [f"{path}:1-{len(source.splitlines())}"
                                    for path, source in c["sources"].items()]} for c in CASES]
        report = grade(responses)
        self.assertTrue(report["mechanical_pass"])
        self.assertEqual(report["semantic_quality"], "not_automatically_verified")
        self.assertTrue(all(r["human_review_required"] for r in report["results"]))
        responses[0]["citations"] = ["candidate.py:3-2", "caller.py:2", "AGENTS.md:1"]
        self.assertIn("source_coverage", grade(responses)["results"][0]["errors"])

    def test_pairs_are_nonvacuous_and_labels_are_blinded(self):
        self.assertEqual(len({c["case_id"] for c in CASES}), 12)
        for pair in {c["pair"] for c in CASES}:
            members = [c for c in CASES if c["pair"] == pair]
            self.assertEqual(len(members), 2)
            self.assertNotEqual(members[0]["sources"], members[1]["sources"])
            if pair != "honesty":
                self.assertEqual({c["expected"]["assessment"] for c in members}, {"supported", "refuted"})
        for task in blinded_tasks():
            self.assertNotIn("expected", task)
            self.assertNotIn("rationale", task)
            self.assertNotIn("claim_to_challenge", task)
        self.assertTrue(all("claim_to_challenge" in task for task in blinded_tasks("challenge")))
        for c in CASES:
            for citation in c["expected"]["required_citations"]:
                path, line = citation.rsplit(":", 1)
                self.assertTrue(0 < int(line) <= len(c["sources"][path].splitlines()))

    def test_grader_rejects_blanket_criticism_and_fake_execution(self):
        # Synthetic responses test the grader, not a model or shipping review behavior.
        responses = [{"case_id": c["case_id"], **{k: c["expected"][k]
                     for k in ("assessment", "execution", "basis")},
                      "argument": dict.fromkeys(ARGUMENT_FIELDS, "synthetic harness sentinel"),
                      "citations": c["expected"]["required_citations"]} for c in CASES]
        self.assertTrue(grade(responses)["mechanical_pass"])
        blanket = copy.deepcopy(responses)
        for r in blanket:
            r["assessment"] = "supported"
        self.assertFalse(grade(blanket)["mechanical_pass"])
        responses[-1]["execution"] = "executed"
        self.assertIn("execution", grade(responses)["results"][-1]["errors"])
        self.assertFalse(grade([])["mechanical_pass"])


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--export", action="store_true")
    parser.add_argument("--phase", choices=("discovery", "challenge"), default="discovery")
    parser.add_argument("--evaluate", type=Path)
    args = parser.parse_args()
    if args.export:
        print(json.dumps(blinded_tasks(args.phase), indent=2))
    elif args.evaluate:
        report = grade(json.loads(args.evaluate.read_text()))
        print(json.dumps(report, indent=2))
        raise SystemExit(0 if report["mechanical_pass"] else 1)
    else:
        unittest.main(argv=[__file__])
