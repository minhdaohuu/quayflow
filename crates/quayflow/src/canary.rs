//! Manual live canary (FR-24 item 3): a synthetic headless job, a swap after its first
//! request, and per-request identity evidence from the Mod. Synthetic content, model haiku.
//! Uses real logins: run it yourself, with consent (`quayflow consent live-canary on`).

use crate::client::Client;
use qf_engine::DataDir;
use serde_json::{json, Value};
use std::io::Read;
use std::time::{Duration, Instant};

pub fn live(dd: &DataDir) -> i32 {
    let c = match Client::connect(dd) {
        Ok(c) => c,
        Err(e) => return crate::fail(e),
    };
    let st = match c.get("/v1/state") {
        Ok(v) => v,
        Err(e) => return crate::fail(e),
    };
    let env = st["envs"]
        .as_array()
        .and_then(|e| e.iter().find(|x| x["id"] == "default"))
        .cloned()
        .unwrap_or_default();
    let from = env["live"]["account"].as_str().unwrap_or("").to_string();
    if env["live"]["known"] != true {
        return crate::fail("the default environment's live login must be a registered account");
    }
    let Some(to) = st["accounts"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|a| a["id"] != from.as_str() && a["health"] == "ok")
        .map(|a| a["id"].as_str().unwrap_or("").to_string())
    else {
        return crate::fail("the live canary needs a second registered account");
    };
    let version = env["inPlace"]["governingVersion"].as_str().unwrap_or("").to_string();
    if let Err(e) = c.post("/v1/canary/start", json!({})) {
        return crate::fail(e);
    }
    let Some(claude) = qf_adapter::process::find_claude() else {
        return crate::fail("Claude Code (claude) not found");
    };
    let prompt = "This is a connectivity check. Run this single Bash command: sleep 10 && echo c1 . After it finishes, reply with exactly OK.";
    let mut child = match qf_adapter::process::hidden(&claude)
        .args([
            "-p",
            prompt,
            "--output-format",
            "json",
            "--model",
            "haiku",
            "--allowedTools",
            "Bash",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(ch) => ch,
        Err(e) => {
            let _ = c.post("/v1/canary/stop", json!({}));
            return crate::fail(e);
        }
    };
    let mut stdout = child.stdout.take().expect("stdout");
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stdout.read_to_string(&mut s);
        s
    });
    let mut swapped_at = 0i64;
    let deadline = Instant::now() + Duration::from_secs(180);
    while swapped_at == 0 && Instant::now() < deadline {
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }
        let steps = c.get("/v1/canary").unwrap_or_default();
        let first_tool = steps["steps"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|s| s["stopReason"] == "tool_use" && s["agent"].is_null());
        if first_tool {
            match c.post("/v1/canary/switch", json!({"to": to})) {
                // The write time, not the reply time: requests are classified by when they started.
                Ok(v) if v["ok"] == true => {
                    swapped_at = v["transition"]["writtenAt"]
                        .as_i64()
                        .unwrap_or_else(qf_model::time::now_ms)
                }
                Ok(v) => {
                    eprintln!("swap refused: {}", v["message"].as_str().unwrap_or(""));
                    break;
                }
                Err(e) => {
                    eprintln!("swap failed: {e}");
                    break;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    let code = child.wait().ok().and_then(|s| s.code()).unwrap_or(-1);
    let out = reader.join().unwrap_or_default();
    std::thread::sleep(Duration::from_millis(1500));
    let run: Value = c.post("/v1/canary/stop", json!({})).unwrap_or_default();
    // Restore the starting account.
    let _ = c.post("/v1/canary/switch", json!({"to": from}));
    let evid: Vec<&Value> = run["steps"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| s["account"].is_string())
        .collect();
    // A request belongs to the side of the swap on which it STARTED: one in flight at the
    // write finishes on the previous account by design.
    let started = |s: &Value| s["startedAt"].as_i64().or_else(|| s["ts"].as_i64()).unwrap_or(0);
    let before: Vec<&&Value> = evid.iter().filter(|s| started(s) < swapped_at).collect();
    let after: Vec<&&Value> = evid.iter().filter(|s| started(s) >= swapped_at).collect();
    let ok_before = !before.is_empty() && before.iter().all(|s| s["account"] == from.as_str());
    let ok_after = !after.is_empty() && after.iter().all(|s| s["account"] == to.as_str());
    let one_json = serde_json::from_str::<Value>(&out)
        .map(|v| v["result"] == "OK")
        .unwrap_or(false);
    let passed = swapped_at > 0 && ok_before && ok_after && code == 0 && one_json;
    outln!(
        "{} live adoption at next request: before swap {} request(s) on #{from}, after swap {} on #{to}",
        if ok_before && ok_after { "PASS" } else { "FAIL" },
        before.len(),
        after.len()
    );
    outln!(
        "{} headless output: exit {code}, one JSON result OK: {one_json}",
        if code == 0 && one_json { "PASS" } else { "FAIL" }
    );
    if !version.is_empty() {
        let _ = c.post(
            "/v1/compat/reset",
            json!({"version": version, "livePass": passed, "detail": format!("manual live canary {}", if passed { "passed" } else { "failed" })}),
        );
    }
    outln!(
        "\n{} for Claude {version}",
        if passed { "CANARY PASSED" } else { "CANARY FAILED" }
    );
    if passed {
        0
    } else {
        1
    }
}
