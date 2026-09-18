"""Real HTTP integration tests; run with the contract's jsonschema dependency.

Build dna/api first, then run python -m unittest discover -s dna/api/tests -v.
HALE_API_BIN can select a separately built service binary. Every Record, port,
issuer and child process belongs to this suite; no organization body is started.
"""

from __future__ import annotations

import json
import os
from pathlib import Path
import tempfile
import unittest
from urllib.parse import urlencode, urlsplit

try:
    from jsonschema import Draft202012Validator
except ImportError as error:
    raise RuntimeError(
        "API integration requires jsonschema; install dna/api/contract/v1/requirements.txt "
        "in a disposable virtual environment and run its Python."
    ) from error

from support import ApiServer, FakeIssuer, Record, free_port, request


API = "/api/hale/v1"
REPO = Path(__file__).resolve().parents[3]


class ReadApiTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.binary = Path(os.environ.get("HALE_API_BIN", REPO / "dna/api/api")).resolve()
        if not cls.binary.is_file():
            raise RuntimeError(f"Build dna/api first, or set HALE_API_BIN; missing {cls.binary}")
        schema_path = REPO / "dna/api/contract/v1/schema.json"
        cls.schema = json.loads(schema_path.read_text())
        Draft202012Validator.check_schema(cls.schema)

    def setUp(self):
        scratch = tempfile.TemporaryDirectory(prefix="hale-dna-api-")
        self.addCleanup(scratch.cleanup)
        self.scratch = Path(scratch.name)
        self.record = Record(self.scratch / "project")
        self.prefix = API + "/applications/" + self.record.identity
        self.server = None

    def start(self, **kwargs):
        self.server = ApiServer(self.binary, self.record, self.scratch / "api.log", **kwargs)
        self.addCleanup(self.server.stop)
        self.server.start()
        return self.server

    def validate(self, response, definition, status=200):
        self.assertEqual(response.status, status, response.body)
        self.assertIn("application/json", response.header("Content-Type"))
        payload = response.json()
        schema = {"$schema": self.schema["$schema"], "$defs": self.schema["$defs"],
                  "$ref": "#/$defs/" + definition}
        errors = sorted(Draft202012Validator(schema).iter_errors(payload), key=lambda e: str(e.path))
        self.assertEqual(errors, [], "\n".join(str(error) for error in errors))
        return payload

    def get(self, resource, query=None, *, definition=None, headers=None):
        path = self.prefix + "/dna/" + resource
        if query:
            path += "?" + urlencode(query)
        response = self.server.get(path, headers=headers)
        return self.validate(response, definition or resource.title() + "Response")

    def item(self, resource, identity):
        payload = self.get(resource, {"id": identity})
        self.assertEqual(payload["data"]["page"]["total"], 1)
        self.assertEqual(len(payload["data"]["items"]), 1)
        return payload["data"]["items"][0]

    def error(self, path, status, *, method="GET", headers=None, body=None):
        response = self.server.get(path, method=method, headers=headers, body=body)
        payload = self.validate(response, "ErrorResponse", status)
        self.assertTrue(payload["error"]["code"])
        return payload

    def assert_source(self, payload):
        self.assertEqual(payload["source"]["record_id"], self.record.identity)
        self.assertEqual(payload["source"]["record_head"], self.record.head)
        self.assertEqual(payload["source"]["record_revision"], str(len(self.record.rows)))

    def test_application_identity_and_read_only_capabilities(self):
        self.start()
        payload = self.validate(self.server.get(API + "/applications"), "ApplicationsResponse")
        self.assert_source(payload)
        applications = payload["data"]["items"]
        self.assertEqual(len(applications), 1)
        self.assertEqual(applications[0]["id"], self.record.identity)
        self.assertEqual(applications[0]["kind"], "dna")
        capability = self.validate(self.server.get(applications[0]["capabilities_url"]), "CapabilitiesResponse")
        self.assert_source(capability)
        data = capability["data"]
        self.assertTrue(data["read_only"])
        self.assertTrue(data["reads"]["practices"])
        self.assertTrue(data["reads"]["reviews"])
        self.assertFalse(any(data["writes"].values()))
        self.assertFalse(data["runtime_observation"])
        self.assertFalse(data["recursive_workflows"])
        self.assertEqual(data["principal"]["mode"], "local")

    def test_native_receipt_unicode_multiline_and_opaque_review_id(self):
        practice = self.record.practice(review_id="org/reviews/決定:α")
        before = self.record.refs()
        self.start()
        payload = self.get("practices")
        self.assert_source(payload)
        item = self.item("practices", practice["digest"])
        for field in ("name", "text", "target", "review_id"):
            self.assertEqual(item[field], practice[field])
        self.assertEqual(item["id"], practice["digest"])
        self.assertEqual(item["digest"], practice["digest"])
        self.assertEqual(item["author"], "org")
        self.assertEqual(item["requester"], "alice")
        self.assertEqual(item["rationale"], "Two lines\nexplain why / pourquoi")
        self.assertTrue(item["text_available"])
        self.assertEqual(item["text_status"], "available")
        review = self.item("reviews", practice["review_id"])
        self.assertEqual(review["id"], practice["review_id"])
        self.assertEqual(review["subject_digest"], practice["digest"])
        self.assertEqual(review["question"], practice["question"])
        self.assertEqual(self.record.refs(), before, "reads must not move any Record or receipt ref")

    def test_approved_review_is_not_activation_and_refusal_is_retained(self):
        practice = self.record.practice()
        self.record.append("review.settled", practice["review_id"], "approve by alice", "dna")
        self.start()
        before_activation = self.item("practices", practice["digest"])
        self.assertEqual(before_activation["state"], "pending")
        self.assertEqual(before_activation["review_state"], "settled")
        self.assertEqual(before_activation["review_settled"], "approve by alice")
        self.assertEqual(before_activation["review_outcome"], "approve")
        self.assertFalse(before_activation["ratified"])
        self.record.append("knowledge.refused", practice["digest"], {
            "digest": practice["digest"], "review_id": practice["review_id"],
            "why": "supersedes a version already retired by another proposal",
        }, "dna")
        refused = self.item("practices", practice["digest"])
        self.assertEqual(refused["state"], "refused")
        self.assertFalse(refused["ratified"])
        self.assertEqual(refused["review_settled"], "approve by alice")
        review = self.item("reviews", practice["review_id"])
        self.assertEqual(review["state"], "settled")
        self.assertEqual(review["settled"], "approve by alice")
        self.assertEqual(review["outcome"], "approve")

    def test_generated_design_practice_uses_its_native_compact_document(self):
        practice = self.record.practice(name="design/record-authority", text="The Record is authoritative.", target="org", design=True)
        self.start()
        item = self.item("practices", practice["digest"])
        self.assertEqual(item["text"], practice["text"])
        self.assertEqual(item["provenance"], "design")
        self.assertTrue(item["text_available"])
        self.assertEqual(item["text_status"], "available")

    def test_ratified_then_declined_stays_ratified_and_retirement_wins(self):
        practice = self.record.practice()
        self.record.append("knowledge.ratified", practice["digest"], {"digest": practice["digest"]})
        self.record.append("knowledge.declined", practice["digest"], {"digest": practice["digest"]})
        self.start()
        self.assertEqual(self.item("practices", practice["digest"])["state"], "ratified")
        self.record.append("knowledge.retired", practice["digest"], {"by": "replacement"})
        self.assertEqual(self.item("practices", practice["digest"])["state"], "retired")

    def test_pagination_has_stable_snapshot_and_rejects_mixed_versions(self):
        expected = {self.record.practice(name=f"practice/{i}", text=f"version {i}")["digest"] for i in range(3)}
        self.start()
        first = self.get("practices", {"limit": 2})
        self.assert_source(first)
        page = first["data"]["page"]
        self.assertEqual((page["limit"], page["offset"], page["total"], page["next_offset"]), (2, 0, 3, 2))
        self.assertEqual(page["snapshot"], self.record.head)
        second = self.get("practices", {"limit": 2, "offset": 2, "snapshot": page["snapshot"]})
        self.assertEqual(second["data"]["page"]["next_offset"], -1)
        self.assertEqual({item["id"] for item in first["data"]["items"] + second["data"]["items"]}, expected)
        self.record.practice(name="practice/new", text="a newly proposed version")
        stale_path = self.prefix + "/dna/practices?" + urlencode({"limit": 2, "offset": 2, "snapshot": page["snapshot"]})
        self.error(stale_path, 409)
        fresh = self.get("practices")
        self.assert_source(fresh)
        self.assertEqual(fresh["data"]["page"]["total"], 4)

    def test_unknown_application_and_native_ids_are_typed_errors(self):
        self.record.practice()
        self.start()
        for path in (API + "/applications/missing/capabilities",
                     API + "/applications//dna/practices",
                     API + "/applications/missing/dna/practices",
                     self.prefix + "/dna/practices?id=sha256%3Amissing",
                     self.prefix + "/dna/reviews?id=org%2Freviews%2Fmissing"):
            with self.subTest(path=path):
                self.error(path, 404)

    def test_malformed_duplicate_and_identity_spoofing_parameters_refuse(self):
        self.record.practice()
        self.start()
        for query in ("limit=0", "limit=101", "limit=-1", "limit=nan",
                      "offset=-1", "offset=1", "limit=1&limit=2", "id=",
                      "id=%", "id=%GG", "id=%00", "id=%FF", "id=%C0%AF",
                      "as=board", "role=board",
                      "position_id=org", "unknown=value"):
            with self.subTest(query=query):
                self.error(self.prefix + "/dna/practices?" + query, 400)

    def test_mutation_methods_cannot_write_or_impersonate(self):
        self.record.practice()
        before = self.record.refs()
        self.start()
        for method in ("POST", "PUT", "PATCH", "DELETE"):
            for suffix in ("/dna/practices", "/dna/reviews", "/commands"):
                with self.subTest(method=method, suffix=suffix):
                    self.error(self.prefix + suffix, 405, method=method,
                               body='{"as":"board","verdict":"approve"}',
                               headers={"Content-Type": "application/json"})
        for legacy_path in ("/api/verdict", "/api/ask"):
            self.error(legacy_path, 405, method="POST", body='{"as":"board"}')
        self.assertEqual(self.record.refs(), before)

    def test_service_restart_preserves_identity_and_projection_without_body(self):
        practice = self.record.practice()
        self.record.append("review.settled", practice["review_id"], "approve by alice")
        self.start()
        before = self.get("practices")
        refs = self.record.refs()
        self.server.stop()
        self.server.start()
        self.assertEqual(self.get("practices"), before)
        self.assertEqual(self.record.refs(), refs)

    def assert_receipt_hidden(self, practice, status):
        item = self.item("practices", practice["digest"])
        self.assertFalse(item["text_available"])
        self.assertEqual(item["text_status"], status)
        self.assertEqual(item["text"], "")
        self.assertEqual(item["rationale"], "")
        review = self.item("reviews", practice["review_id"])
        self.assertFalse(review["text_available"])
        self.assertEqual(review["question"], "")
        for resource in ("practices", "reviews"):
            self.assertNotIn(practice["text"], json.dumps(self.get(resource), ensure_ascii=False))

    def hidden_item_or_unavailable(self, resource, identity, *canaries,
                                   text_statuses=("source_unavailable", "protected")):
        """Uncertain visibility may withhold metadata too, but never reveal text."""
        path = self.prefix + "/dna/" + resource + "?" + urlencode({"id": identity})
        response = self.server.get(path)
        for canary in canaries:
            self.assertNotIn(canary, response.body)
        if response.status == 503:
            self.validate(response, "ErrorResponse", 503)
            return None
        payload = self.validate(response, resource.title() + "Response")
        self.assertEqual(payload["data"]["page"]["total"], 1)
        self.assertEqual(len(payload["data"]["items"]), 1)
        item = payload["data"]["items"][0]
        self.assertFalse(item["text_available"])
        self.assertIn(item["text_status"], text_statuses)
        return item

    def assert_uncertain_practice_hidden(self, practice):
        item = self.hidden_item_or_unavailable(
            "practices", practice["digest"], practice["text"], "VISIBILITY-DECIDER-CANARY")
        if item is not None:
            self.assertEqual(item["text"], "")
            self.assertEqual(item["rationale"], "")
            self.assertEqual(item["review_settled"], "")
            self.assertEqual(item["review_outcome"], "approve")
        review = self.hidden_item_or_unavailable(
            "reviews", practice["review_id"], practice["text"], "VISIBILITY-DECIDER-CANARY")
        if review is not None:
            self.assertEqual(review["question"], "")
            self.assertEqual(review["settled"], "")
            self.assertEqual(review["outcome"], "approve")

    def test_redacted_receipt_and_question_stay_hidden_with_stale_blob(self):
        practice = self.record.practice(text="REDACTED-CANARY: customer private data")
        self.record.append("review.settled", practice["review_id"], "approve by DECIDER-CANARY")
        self.record.append("review.reasoned", practice["review_id"], "REASONED-CANARY: private justification")
        self.record.append("receipt.redacted", practice["digest"], {"by": "alice", "why": "expired", "policy": "retention"})
        self.start()
        self.assert_receipt_hidden(practice, "redacted")
        hidden = self.item("practices", practice["digest"])
        self.assertEqual(hidden["review_outcome"], "approve")
        self.assertEqual(hidden["review_settled"], "")
        review = self.item("reviews", practice["review_id"])
        self.assertEqual(review["outcome"], "approve")
        self.assertEqual(review["settled"], "")
        # The content still physically exists: omission is authority-based.
        self.assertIn("REDACTED-CANARY", self.record.git("cat-file", "-p", "refs/dna/receipts/" + practice["digest"][7:]))
        self.assertNotIn("REASONED-CANARY", self.server.get(self.prefix + "/dna/reviews").body)
        self.assertNotIn("DECIDER-CANARY", self.server.get(self.prefix + "/dna/reviews").body)

    def test_classified_and_withheld_receipts_never_leak_stale_local_text(self):
        cases = []
        for index, kind in enumerate(("receipt.classified", "receipt.withheld")):
            practice = self.record.practice(name=f"protected/{index}", text="PROTECTED-CANARY-" + kind)
            self.record.append(kind, practice["digest"], {"class": "confidential", "store": "knowledge"})
            cases.append(practice)
        self.start()
        for practice in cases:
            with self.subTest(digest=practice["digest"]):
                self.assert_receipt_hidden(practice, "protected")

    def test_adoption_without_ledger_visibility_withholds_receipt_text(self):
        practice = self.record.practice(text="LEDGER-VISIBILITY-CANARY")
        self.record.append("ledger.adopted", "ledger", {"routing": 1, "checkpoint": self.record.head})
        self.start()
        self.assert_receipt_hidden(practice, "source_unavailable")
        self.assert_source(self.get("practices"))

    def test_abandoning_ledger_does_not_restore_unproven_receipt_visibility(self):
        practice = self.record.practice(text="ABANDONED-LEDGER-CANARY")
        self.record.append("review.settled", practice["review_id"], "approve by VISIBILITY-DECIDER-CANARY")
        self.record.append("ledger.adopted", "ledger", {"routing": 1, "checkpoint": self.record.head})
        self.record.append("ledger.abandoned", "ledger", {"why": "keeper unavailable", "by": "alice"})
        # No Record restriction marker was copied back from the former Ledger.
        # Its absence cannot authorize the stale blob that remains in this clone.
        self.assertIn(practice["text"], self.record.git("cat-file", "-p", "refs/dna/receipts/" + practice["digest"][7:]))
        self.start()
        self.assert_uncertain_practice_hidden(practice)

    def test_malformed_receipt_classification_does_not_expose_stale_blob(self):
        cases = (
            ("receipt.classified", {}),
            ("receipt.classified", {"class": None}),
            ("receipt.classified", {"class": 42}),
            ("receipt.classified", {"class": {"name": "confidential"}}),
            ("receipt.classified", {"class": "unknown-class"}),
            ("receipt.withheld", {}),
        )
        for index, (kind, body) in enumerate(cases):
            with self.subTest(kind=kind, body=body):
                # An independently malformed Record must not mask another case.
                self.record = Record(self.scratch / f"classification-{index}")
                self.prefix = API + "/applications/" + self.record.identity
                practice = self.record.practice(text=f"UNCERTAIN-CLASS-CANARY-{index}")
                self.record.append("review.settled", practice["review_id"], "approve by VISIBILITY-DECIDER-CANARY")
                self.record.append(kind, practice["digest"], body)
                self.assertIn(practice["text"], self.record.git("cat-file", "-p", "refs/dna/receipts/" + practice["digest"][7:]))
                self.start()
                try:
                    self.assert_uncertain_practice_hidden(practice)
                finally:
                    self.server.stop()

    def test_unadopted_nonknowledge_review_keeps_its_declared_question_and_decision(self):
        review_id = "operations/nonknowledge-review"
        question = "Approve the service change?\nScope: org/support — exact candidate"
        subject = "sha256:" + "a" * 64
        self.record.append("review.requested", "review:" + review_id, {
            "question": question, "subject_digest": subject,
            "required_authority": "board", "author": "org",
        })
        self.record.append("review.settled", review_id, "approve by alice")
        self.start()
        review = self.item("reviews", review_id)
        self.assertEqual(review["knowledge_digest"], "")
        self.assertEqual(review["subject_digest"], subject)
        self.assertEqual(review["text_status"], "not_applicable")
        self.assertFalse(review["text_available"], "no canonical practice receipt was attached")
        self.assertEqual(review["question"], question)
        self.assertEqual(review["settled"], "approve by alice")
        self.assertEqual(review["outcome"], "approve")

    def test_unlinked_review_text_is_withheld_when_ledger_visibility_is_unknown(self):
        digest = self.record.receipt_text("UNLINKED-RECEIPT-CANARY")
        review_id = "operations/review-without-knowledge-link"
        self.record.append("review.requested", "review:" + review_id, {
            "question": "UNLINKED-QUESTION-CANARY", "subject_digest": digest,
            "required_authority": "board", "author": "org",
        })
        self.record.append("review.settled", review_id, "approve by VISIBILITY-DECIDER-CANARY")
        self.record.append("ledger.adopted", "ledger", {"routing": 1, "checkpoint": self.record.head})
        self.start()
        review = self.hidden_item_or_unavailable(
            "reviews", review_id, "UNLINKED-QUESTION-CANARY", "VISIBILITY-DECIDER-CANARY")
        if review is not None:
            self.assertEqual(review["question"], "")
            self.assertEqual(review["settled"], "")
            self.assertEqual(review["outcome"], "approve")

    def test_inconsistent_review_receipt_links_cannot_bypass_redaction(self):
        visible = self.record.practice(name="visible/practice", text="A visible canonical practice")
        redacted = self.record.practice(name="redacted/practice", text="MISMATCHED-SUBJECT-CANARY")
        self.record.append("receipt.redacted", redacted["digest"], {"by": "alice", "why": "expired", "policy": "retention"})
        review_id = "review/inconsistent-receipt-links"
        self.record.append("review.requested", "review:" + review_id, {
            "question": redacted["text"], "subject_digest": redacted["digest"],
            "knowledge_digest": visible["digest"],
            "required_authority": "board", "author": "org",
        })
        self.record.append("review.settled", review_id, "approve by VISIBILITY-DECIDER-CANARY")
        self.start()
        self.assertEqual(self.item("practices", visible["digest"])["text"], visible["text"])
        review = self.hidden_item_or_unavailable(
            "reviews", review_id, redacted["text"], "VISIBILITY-DECIDER-CANARY",
            text_statuses=("invalid_document", "source_unavailable", "redacted", "protected"))
        if review is not None:
            self.assertEqual(review["question"], "")
            self.assertEqual(review["settled"], "")
            self.assertEqual(review["outcome"], "approve")

    def test_missing_and_digest_mismatched_receipts_are_not_empty_documents(self):
        missing = self.record.practice(name="missing", text="MISSING-CANARY")
        self.record.git("update-ref", "-d", "refs/dna/receipts/" + missing["digest"][7:])
        wrong = self.record.practice(name="wrong", text="ORIGINAL-CANARY")
        blob = self.record.git("hash-object", "-w", "--stdin", input='{"text":"TAMPERED-CANARY"}')
        self.record.git("update-ref", "refs/dna/receipts/" + wrong["digest"][7:], blob)
        self.start()
        self.assert_receipt_hidden(missing, "missing")
        self.assert_receipt_hidden(wrong, "digest_mismatch")
        self.assertNotIn("TAMPERED-CANARY", self.server.get(self.prefix + "/dna/practices").body)

    def test_valid_receipt_hash_does_not_make_malformed_document_valid(self):
        malformed = '{"kind":"practice","text":"INVALID-DOCUMENT-CANARY"'
        digest = self.record.receipt_text(malformed)
        review_id = "k:" + digest[7:19]
        self.record.append("knowledge.proposed", digest, {
            "digest": digest, "review_id": review_id, "kind": "practice",
            "name": "invalid/document", "author": "org", "target": "org", "class": "initiative",
        })
        self.record.append("review.requested", "review:" + review_id, {
            "question": "INVALID-DOCUMENT-CANARY", "subject_digest": digest,
            "knowledge_digest": digest, "required_authority": "board", "author": "org",
        })
        self.start()
        self.assert_receipt_hidden({"digest": digest, "review_id": review_id,
                                   "text": "INVALID-DOCUMENT-CANARY"}, "invalid_document")
        # The Record is valid and remains readable; only this receipt is invalid.
        self.assert_source(self.get("practices"))

    def test_missing_record_is_unavailable_not_empty_success(self):
        self.start()
        self.record.git("update-ref", "-d", "refs/dna/journal")
        self.error(API + "/applications", 503)

    def test_malformed_record_is_unavailable_not_partial_success(self):
        self.record.practice()
        self.start()
        self.record.commit_text("not a JSON journal\n")
        self.error(API + "/applications", 503)

    def configure_oidc(self):
        issuer = FakeIssuer().start()
        self.addCleanup(issuer.stop)
        port = free_port()
        settings = {
            "dna.principal": "oidc", "dna.oidc.issuer": issuer.base,
            "dna.oidc.client": "dna-api-tests",
            "dna.oidc.redirect": f"http://127.0.0.1:{port}/auth/callback",
            "dna.oidc.member": "alice-sub=alice", "dna.oidc.board": "alice",
        }
        for key, value in settings.items():
            self.record.git("config", key, value)
        self.start(port=port, env={"HALE_DNA_OIDC_SECRET": "fake-secret"})
        return issuer

    def sign_in(self, login_hint=None):
        login = self.server.get("/auth/login")
        self.assertEqual(login.status, 302, login.body)
        signin = login.header("Set-Cookie").split(";", 1)[0]
        authorization = urlsplit(login.header("Location"))
        target = authorization.path + "?" + authorization.query
        if login_hint:
            target += "&" + urlencode({"login_hint": login_hint})
        authorized = request(authorization.port, target)
        self.assertEqual(authorized.status, 302)
        callback = urlsplit(authorized.header("Location"))
        callback_path = callback.path + "?" + callback.query
        landed = self.server.get(callback_path, headers={"Cookie": signin})
        return landed

    def test_oidc_refuses_anonymous_forged_and_unmapped_sessions(self):
        self.record.practice()
        before = self.record.refs()
        self.configure_oidc()
        self.error(API + "/applications", 401)
        self.error(self.prefix + "/dna/practices", 401, headers={"Cookie": "dna_session=forged"})
        unmapped = self.sign_in("unmapped-sub")
        self.assertEqual(unmapped.status, 403, unmapped.body)
        self.assertNotIn("dna_session=", unmapped.header("Set-Cookie"))
        self.assertEqual(self.record.refs(), before)

    def test_incomplete_oidc_configuration_cannot_fall_back_to_local(self):
        self.record.git("config", "dna.principal", "oidc")
        before = self.record.refs()
        with self.assertRaisesRegex(AssertionError, "API exited before listening"):
            self.start()
        self.assertEqual(self.record.refs(), before)

    def test_oidc_session_reads_then_logout_and_restart_revoke_session(self):
        practice = self.record.practice()
        issuer = self.configure_oidc()
        logged_in = self.sign_in()
        self.assertEqual(logged_in.status, 302, logged_in.body)
        self.assertIn("HttpOnly", logged_in.header("Set-Cookie"))
        cookie = logged_in.header("Set-Cookie").split(";", 1)[0]
        capabilities = self.validate(self.server.get(self.prefix + "/capabilities", headers={"Cookie": cookie}), "CapabilitiesResponse")
        self.assertEqual(capabilities["data"]["principal"], {"mode": "oidc", "name": "alice"})
        payload = self.get("practices", headers={"Cookie": cookie})
        self.assertEqual(payload["data"]["items"][0]["digest"], practice["digest"])
        self.assertEqual(issuer.exchanges, 1)
        self.assertEqual(self.server.get("/auth/logout", headers={"Cookie": cookie}).status, 302)
        self.error(API + "/applications", 401, headers={"Cookie": cookie})
        logged_in = self.sign_in()
        cookie = logged_in.header("Set-Cookie").split(";", 1)[0]
        self.server.stop()
        self.server.start()
        self.error(API + "/applications", 401, headers={"Cookie": cookie})


if __name__ == "__main__":
    unittest.main()
