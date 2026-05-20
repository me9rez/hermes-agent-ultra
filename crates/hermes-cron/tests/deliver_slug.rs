//! Regression: cron deliver JSON uses Python platform slugs (`wecom`), not `we_com`.

use hermes_cron::{CronJob, DeliverTarget};

#[test]
fn per_job_json_wecom_deliver_object() {
    let contents = r#"{
  "id": "d0b0cf77-bd3f-4ab7-9ac6-b4553cdfb76e",
  "schedule": "every 2h",
  "prompt": "鍠濇按",
  "deliver": { "target": "wecom" },
  "origin": { "platform": "wecom", "chat_id": "wrPMNBUgAAxFJsvKPM6tTJ2csX586dqQ" },
  "created_at": "2026-05-17T17:27:05.435702300Z"
}"#;
    let job: CronJob = serde_json::from_str(contents).expect("wecom job file");
    assert_eq!(
        job.deliver.as_ref().map(|d| d.target),
        Some(DeliverTarget::WeCom)
    );
}

#[test]
fn deliver_string_and_object_roundtrip_wecom() {
    let ts = "2026-05-17T17:27:05Z";
    let object = format!(
        r#"{{"id":"x","schedule":"every 2h","prompt":"p","created_at":"{ts}","deliver":{{"target":"wecom"}}}}"#
    );
    let job: CronJob = serde_json::from_str(&object).expect("object deliver");
    assert_eq!(
        job.deliver.as_ref().map(|d| d.target),
        Some(DeliverTarget::WeCom)
    );

    let string = format!(
        r#"{{"id":"y","schedule":"every 2h","prompt":"p","created_at":"{ts}","deliver":"wecom"}}"#
    );
    let job: CronJob = serde_json::from_str(&string).expect("string deliver");
    assert_eq!(
        job.deliver.as_ref().map(|d| d.target),
        Some(DeliverTarget::WeCom)
    );

    let json = serde_json::to_string(&job).unwrap();
    assert!(json.contains(r#""target":"wecom""#));
    assert!(!json.contains("we_com"));
}