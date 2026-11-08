//! Plain-text views for the terminal.

use serde_json::Value;

fn age(s: i64) -> String {
    if s < 90 {
        format!("{s} s")
    } else if s < 5400 {
        format!("{} min", s / 60)
    } else {
        format!("{} h", s / 3600)
    }
}

pub fn status(v: &Value) {
    let e = &v["engine"];
    outln!(
        "Quayflow engine {} (pid {}) · {}",
        e["version"].as_str().unwrap_or("?"),
        e["pid"],
        e["integrity"].as_str().unwrap_or("")
    );
    if let Some(h) = e["exeSha256"].as_str().filter(|h| !h.is_empty()) {
        outln!("Build SHA-256: {h}");
    }
    if let Some(s) = e["simulation"].as_str() {
        outln!("[warn] SIMULATION MODE: usage and profile calls go to the fake API at {s} (QUAYFLOW_SIM_API)");
    }
    let labels: Vec<(String, String)> = v["accounts"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|a| {
            (
                a["id"].as_str().unwrap_or("").to_string(),
                a["label"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect();
    let label = |id: &Value| {
        let id = id.as_str().unwrap_or("");
        labels
            .iter()
            .find(|(i, _)| i == id)
            .map(|(_, l)| l.clone())
            .unwrap_or_else(|| format!("#{id}"))
    };
    for env in v["envs"].as_array().into_iter().flatten() {
        outln!();
        let live = &env["live"];
        let who = if live["signedOut"] == true {
            "signed out".to_string()
        } else if live["known"] == true {
            live["label"].as_str().unwrap_or("?").to_string()
        } else {
            format!("unregistered login #{}", live["account"].as_str().unwrap_or("?"))
        };
        outln!(
            "Environment {} · live: {who} · mode {} · groups {}",
            env["id"].as_str().unwrap_or("?"),
            env["mode"].as_str().unwrap_or("?"),
            env["governingGroups"]
                .as_array()
                .map(|g| g.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", "))
                .unwrap_or_default()
        );
        outln!("  decision: {}", env["decision"].as_str().unwrap_or("-"));
        if let Some(p) = env["pin"].as_object() {
            outln!(
                "  pinned: {} until {} (quayflow unpin)",
                label(&p["account"]),
                qf_model::time::to_rfc3339(p["until"].as_i64().unwrap_or(0))
            );
        }
        if let Some(s) = env["suggestion"].as_object() {
            outln!(
                "  suggestion: switch to {} — {} (quayflow accept)",
                label(&s["to"]),
                s["reason"].as_str().unwrap_or("")
            );
        }
        if let Some(t) = env["transition"].as_object() {
            outln!(
                "  transition: {} -> {} · {}",
                label(&t["from"]),
                label(&t["to"]),
                t["state"].as_str().unwrap_or("?")
            );
        }
        for n in env["notices"].as_array().into_iter().flatten() {
            outln!(
                "  [{}] {}",
                n["level"].as_str().unwrap_or(""),
                n["text"].as_str().unwrap_or("")
            );
        }
        let sessions = env["sessions"].as_array().cloned().unwrap_or_default();
        outln!(
            "  sessions on this login ({}; other clients such as editor extensions may not be listed):",
            sessions.len()
        );
        for s in &sessions {
            let ad = env["adoption"]
                .as_array()
                .into_iter()
                .flatten()
                .rev()
                .find(|a| a["sessionId"] == s["sessionId"])
                .map(|a| match a["state"].as_str() {
                    Some("pending_next_request") => " · Pending next request".to_string(),
                    Some("adopted") => " · adopted".to_string(),
                    Some("mismatch") => " · did not adopt the switch (see notices)".to_string(),
                    _ => String::new(),
                })
                .unwrap_or_default();
            outln!(
                "    {} · {} · group {} · Claude {}{ad}",
                s["name"]
                    .as_str()
                    .map(String::from)
                    .unwrap_or_else(|| format!("pid {}", s["pid"])),
                s["status"].as_str().unwrap_or("?"),
                s["group"].as_str().unwrap_or("?"),
                s["version"].as_str().unwrap_or("?")
            );
        }
    }
    outln!("\nAccounts:");
    for a in v["accounts"].as_array().into_iter().flatten() {
        // A stale reading is never printed as a current figure.
        let usage = match a["usage"].as_object() {
            Some(u) if u["fresh"] == true => {
                let b = &u["binding"];
                format!(
                    "{:.0}% {} ({} old)",
                    b["pct"].as_f64().unwrap_or(0.0),
                    qf_model::window_name(b["key"].as_str().unwrap_or("?")),
                    age(u["ageS"].as_i64().unwrap_or(0))
                )
            }
            Some(u) => format!("usage unknown (last read {} ago)", age(u["ageS"].as_i64().unwrap_or(0))),
            None => "usage unknown".into(),
        };
        let note = a["usageNote"].as_str().map(|n| format!(" — {n}")).unwrap_or_default();
        let live = if a["liveIn"].as_array().is_some_and(|x| !x.is_empty()) {
            "▶"
        } else {
            " "
        };
        let elig = a["eligibility"]["reason"].as_str().unwrap_or("");
        outln!(
            "{live} {:<16} {:<12} {:<28} {}{note}",
            a["label"].as_str().unwrap_or("?"),
            a["kind"].as_str().unwrap_or("?"),
            usage,
            if a["health"] == "ok" {
                elig.to_string()
            } else {
                format!("[{}] {elig}", a["health"].as_str().unwrap_or(""))
            }
        );
    }
}

/// An account's label from a state view, or `#id` when it is not registered.
fn account_label(v: &Value, id: &str) -> String {
    v["accounts"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|x| x["id"] == id)
        .and_then(|x| x["label"].as_str())
        .map(String::from)
        .unwrap_or_else(|| format!("#{id}"))
}

pub fn groups(v: &Value) {
    let default = v["defaultGroup"].as_str().unwrap_or("");
    for g in v["groups"].as_array().into_iter().flatten() {
        let name = g["name"].as_str().unwrap_or("?");
        outln!(
            "{}{} · mode {} · threshold {}% · margin {} · home {}{}",
            name,
            if name == default { " (default)" } else { "" },
            g["mode"].as_str().unwrap_or("?"),
            g["threshold"],
            g["margin"],
            g["preferred"]
                .as_str()
                .map(|p| account_label(v, p))
                .unwrap_or_else(|| "none".into()),
            if g["mixed"] == true {
                if g["mixedCovered"] == true {
                    " · mixed identities (disclosure accepted)"
                } else {
                    " · mixed identities"
                }
            } else {
                ""
            }
        );
        for a in g["accounts"].as_array().into_iter().flatten() {
            outln!("    {}", account_label(v, a.as_str().unwrap_or("")));
        }
    }
}

pub fn lanes(v: &Value) {
    let list = v.as_array().cloned().unwrap_or_default();
    if list.is_empty() {
        outln!("No lanes. A lane is a separate Claude environment for sessions that need a different account (quayflow lanes create <name>).");
    }
    for l in list {
        outln!(
            "{} · {} · links {} · check {} · {} session(s)",
            l["name"].as_str().unwrap_or("?"),
            match (l["liveLabel"].as_str(), l["live"].as_str()) {
                (Some(label), _) => format!("live: {label}"),
                (None, Some(x)) => format!("live: #{x}"),
                _ => "no login".into(),
            },
            if l["linksOk"] == true { "ok" } else { "need repair" },
            if l["canaryPassed"] == true {
                "passed"
            } else {
                "not passed"
            },
            l["sessions"]
        );
        outln!("    {}", l["configDir"].as_str().unwrap_or(""));
        for d in l["linkDetail"].as_array().into_iter().flatten() {
            outln!("    - {}", d.as_str().unwrap_or(""));
        }
    }
}

pub fn compat(v: &Value) {
    for r in v.as_array().into_iter().flatten() {
        outln!(
            "Claude {} · static {} · live {} {}",
            r["version"].as_str().unwrap_or("?"),
            if r["staticPass"] == true { "PASS" } else { "FAIL" },
            match r["livePass"].as_bool() {
                Some(true) => "PASS",
                Some(false) => "FAIL",
                None => "not yet",
            },
            r["staticDetail"]
                .as_array()
                .map(|d| d.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join("; "))
                .unwrap_or_default()
        );
    }
}
