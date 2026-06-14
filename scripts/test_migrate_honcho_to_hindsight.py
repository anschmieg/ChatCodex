import importlib.util
import tempfile
import unittest
from unittest import mock
from datetime import datetime, timezone
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("migrate_honcho_to_hindsight.py")
SPEC = importlib.util.spec_from_file_location("migration", MODULE_PATH)
assert SPEC and SPEC.loader
migration = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(migration)


class Item:
    def __init__(self, **values):
        self.__dict__.update(values)


class HindsightClient:
    profile = "hermes"
    _manager = Item(get_url=lambda profile: "http://127.0.0.1:9177")

    def __init__(self):
        self.retain_calls = 0

    def retain(self, **kwargs):
        self.retain_calls += 1
        return Item(operation_id="operation-1", operation_ids=None)


class MigrationFormattingTests(unittest.TestCase):
    def test_safe_id_is_stable_and_bounded(self):
        first = migration.safe_id("session/with spaces")
        second = migration.safe_id("session/with spaces")
        self.assertEqual(first, second)
        self.assertNotIn("/", first)
        self.assertLessEqual(len(migration.safe_id("x" * 500)), 96)

    def test_redacts_nested_secrets(self):
        value = {"apiKey": "secret", "nested": {"token": "secret", "safe": 1}}
        self.assertEqual(
            migration.redact_config(value),
            {"apiKey": "<redacted>", "nested": {"token": "<redacted>", "safe": 1}},
        )

    def test_session_render_preserves_roles_and_timestamps(self):
        session = Item(
            id="abc",
            created_at=datetime(2026, 1, 1, tzinfo=timezone.utc),
        )
        messages = [
            Item(
                peer_id="alice",
                content="I prefer concise answers.",
                created_at=datetime(2026, 1, 1, 1, tzinfo=timezone.utc),
            ),
            Item(
                peer_id="hermes",
                content="Understood.",
                created_at=datetime(2026, 1, 1, 1, 1, tzinfo=timezone.utc),
            ),
        ]
        rendered = migration.render_session(session, messages, "alice", "hermes")
        self.assertIn("## User", rendered)
        self.assertIn("## Assistant", rendered)
        self.assertIn("I prefer concise answers.", rendered)
        self.assertIn("2026-01-01T01:00:00Z", rendered)

    def test_call_compatible_drops_newer_sdk_keywords(self):
        def legacy_method(page=1):
            return page

        self.assertEqual(
            migration.call_compatible(legacy_method, page=2, size=100, reverse=True),
            2,
        )

    def test_session_render_uses_first_message_when_session_has_no_timestamp(self):
        session = Item(id="legacy")
        messages = [
            Item(
                peer_id="alice",
                content="Legacy message",
                created_at=datetime(2025, 5, 1, tzinfo=timezone.utc),
            )
        ]
        rendered = migration.render_session(session, messages, "alice", "hermes")
        self.assertIn("Created: 2025-05-01T00:00:00Z", rendered)

    def test_retain_document_waits_for_async_operation(self):
        client = HindsightClient()
        with tempfile.TemporaryDirectory() as directory:
            checkpoint_path = Path(directory) / "state.json"
            with (
                mock.patch.object(
                    migration,
                    "document_status",
                    side_effect=[
                        ("absent", None),
                        ("complete", {"memory_unit_count": 3}),
                    ],
                ),
                mock.patch.object(
                    migration,
                    "operation_status",
                    return_value={"status": "completed"},
                ),
            ):
                result = migration.retain_document(
                    client,
                    "bank",
                    "document",
                    "content",
                    "context",
                    ["source:honcho"],
                    checkpoint={"documents": {}},
                    checkpoint_path=checkpoint_path,
                    position=1,
                    total=1,
                )
        self.assertEqual(result, "completed")
        self.assertEqual(client.retain_calls, 1)

    def test_retain_document_skips_matching_remote_document(self):
        client = HindsightClient()
        with tempfile.TemporaryDirectory() as directory:
            checkpoint_path = Path(directory) / "state.json"
            with mock.patch.object(
                migration,
                "document_status",
                return_value=("complete", {"memory_unit_count": 7}),
            ):
                result = migration.retain_document(
                    client,
                    "bank",
                    "document",
                    "content",
                    "context",
                    ["source:honcho"],
                    checkpoint={"documents": {}},
                    checkpoint_path=checkpoint_path,
                    position=1,
                    total=1,
                )
        self.assertEqual(result, "skipped")
        self.assertEqual(client.retain_calls, 0)


if __name__ == "__main__":
    unittest.main()
