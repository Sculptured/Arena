use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

#[test]
fn describe_over_stdio() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_arena-retail"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":1,"method":"describe","params":{{}}}}"#
    )
    .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    drop(stdin);
    child.wait().unwrap();
    let v: serde_json::Value = serde_json::from_str(&line).unwrap();
    let r = &v["result"];
    assert_eq!(r["name"], "retail");
    assert_eq!(r["tasks"].as_array().unwrap().len(), 10);
    let effect = |n: &str| {
        r["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == n)
            .unwrap()["effect"]
            .clone()
    };
    assert_eq!(effect("cancel_pending_order"), "write");
    assert_eq!(effect("get_order_details"), "read");
}
