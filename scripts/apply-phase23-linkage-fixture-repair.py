from pathlib import Path

root = Path(__file__).resolve().parents[1]
path = root / "crates/homeserver-service/src/audio_runtime.rs"
text = path.read_text(encoding="utf-8")
old = '''        connection
            .execute(
                "INSERT INTO agent_messages(message_id,thread_id,role,mode,content,context_json,created_at_utc) VALUES('msg_link','thread_link','user','ask','hello','{}','2026-08-01T00:00:02.000Z')",
                [],
            )
            .expect("message");
        connection
            .execute(
                "UPDATE audio_segments SET state='committed',transcript='hello',linked_message_id='msg_link',updated_at_utc='2026-08-01T00:00:02.000Z',finalized_at_utc='2026-08-01T00:00:02.000Z' WHERE segment_id='audseg_link'",
                [],
            )
            .expect("verified link");
'''
new = '''        connection
            .execute(
                "INSERT INTO agent_messages(message_id,thread_id,role,mode,content,context_json,created_at_utc) VALUES('msg_missing_thread','thread_missing','user','ask','hello','{}','2026-08-01T00:00:02.000Z')",
                [],
            )
            .expect("message without retained thread");
        let missing_thread_error = connection
            .execute(
                "UPDATE audio_segments SET state='committed',transcript='hello',linked_message_id='msg_missing_thread',updated_at_utc='2026-08-01T00:00:02.000Z',finalized_at_utc='2026-08-01T00:00:02.000Z' WHERE segment_id='audseg_link'",
                [],
            )
            .expect_err("message thread must exist");
        assert!(missing_thread_error
            .to_string()
            .contains("invalid Phase 23 transcript linkage"));
        connection
            .execute(
                "INSERT INTO agent_threads(thread_id,title,state,created_at_utc,updated_at_utc) VALUES('thread_link','Linked audio','active','2026-08-01T00:00:00.000Z','2026-08-01T00:00:00.000Z')",
                [],
            )
            .expect("message thread");
        connection
            .execute(
                "INSERT INTO agent_messages(message_id,thread_id,role,mode,content,context_json,created_at_utc) VALUES('msg_link','thread_link','user','ask','hello','{}','2026-08-01T00:00:02.000Z')",
                [],
            )
            .expect("message");
        connection
            .execute(
                "UPDATE audio_segments SET state='committed',transcript='hello',linked_message_id='msg_link',updated_at_utc='2026-08-01T00:00:02.000Z',finalized_at_utc='2026-08-01T00:00:02.000Z' WHERE segment_id='audseg_link'",
                [],
            )
            .expect("verified link");
'''
if text.count(old) != 1:
    raise SystemExit(f"Expected one Phase 23 linkage fixture block, found {text.count(old)}")
path.write_text(text.replace(old, new, 1), encoding="utf-8")
print("Phase 23A linkage fixture repair applied.")
