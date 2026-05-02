//! Wiremock-backed integration tests for `HttpRunner`.
//!
//! Each test boots its own in-process `MockServer` and points an
//! `HttpRunner` at it. This is the only place where we exercise the
//! actual on-the-wire HTTP request/response cycle: request body shape,
//! headers, and how 4xx/2xx flow through the curl-shelling code path.
//!
//! Why integration test (and not `#[cfg(test)]` unit test): wiremock's
//! `MockServer::start()` requires a tokio runtime, and we don't want to
//! drag tokio into the production-side dependency graph.
//!
//! We use `#[tokio::test(flavor = "current_thread")]` to keep the
//! runtime lightweight. `HttpRunner::run` itself is sync (it shells to
//! curl), and curl talks to the mock server like any other HTTP target,
//! so no async juggling is required at the runner side.
//!
//! Wiremock 0.6's `Match` trait is implemented for any
//! `Fn(&Request) -> bool`, so request-body inspection is done with
//! plain closures rather than the (separate) `body_json_schema`
//! deserialize-based matcher.

use coral_runner::{HttpRunner, Prompt, Runner, RunnerError};
use serde_json::Value;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

/// Fresh prompt for tests that don't care about prompt details.
fn user_prompt(s: &str) -> Prompt {
    Prompt {
        user: s.into(),
        model: Some("test-model".into()),
        ..Default::default()
    }
}

/// 1. Verifies the runner sends the expected chat-completions JSON shape
///    (model, messages array, stream:false) and parses the canned
///    response back into `RunOutput.stdout`.
///
///    The closure-matcher inspects the parsed body and returns true only
///    when all required fields look right. If it returns false the mock
///    won't match and the runner will see a 404 — which surfaces as Err
///    and fails the `expect("expected Ok")` below loudly.
#[tokio::test(flavor = "current_thread")]
async fn http_runner_sends_correct_chat_completions_shape() {
    let server = MockServer::start().await;

    let body_matcher = |req: &Request| -> bool {
        let body: Value = match req.body_json() {
            Ok(v) => v,
            Err(_) => return false,
        };
        // model field present and a string
        let model_ok = body.get("model").and_then(|m| m.as_str()).is_some();
        // stream must be exactly false
        let stream_ok = body.get("stream") == Some(&Value::Bool(false));
        // messages must be an array containing system + user roles
        let messages = match body.get("messages").and_then(|m| m.as_array()) {
            Some(arr) => arr,
            None => return false,
        };
        let roles: Vec<&str> = messages
            .iter()
            .filter_map(|m| m.get("role").and_then(|r| r.as_str()))
            .collect();
        let roles_ok = roles.contains(&"system") && roles.contains(&"user");
        model_ok && stream_ok && roles_ok
    };

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_matcher)
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "choices": [
                { "message": { "content": "mock reply", "role": "assistant" } }
            ]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let endpoint = format!("{}/v1/chat/completions", server.uri());
    let runner = HttpRunner::new(endpoint);

    let prompt = Prompt {
        system: Some("you are a helpful assistant".into()),
        user: "say hi".into(),
        model: Some("test-model".into()),
        ..Default::default()
    };
    let out = runner.run(&prompt).expect("expected Ok from runner");
    assert_eq!(
        out.stdout, "mock reply",
        "stdout should be the parsed `choices[0].message.content`"
    );
}

/// 2. With `.with_api_key("test-key")`, the request must carry an
///    `Authorization: Bearer test-key` header. The `header` matcher
///    requires the header to be present with that exact value; if curl
///    omits the header, the mock returns 404 and the runner returns Err.
#[tokio::test(flavor = "current_thread")]
async fn http_runner_sends_authorization_header_when_set() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("Authorization", "Bearer test-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "choices": [{ "message": { "content": "ok", "role": "assistant" } }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let endpoint = format!("{}/v1/chat/completions", server.uri());
    let runner = HttpRunner::new(endpoint).with_api_key("test-key");

    let out = runner
        .run(&user_prompt("hi"))
        .expect("expected Ok — auth header must reach the server");
    assert_eq!(out.stdout, "ok");
}

/// 3. Without `.with_api_key(...)`, the request must NOT include an
///    Authorization header. The closure-matcher returns true only when
///    the Authorization header is absent. If curl accidentally sent one,
///    the mock won't match and the runner returns Err.
#[tokio::test(flavor = "current_thread")]
async fn http_runner_omits_authorization_when_no_key() {
    let server = MockServer::start().await;

    let no_auth_header = |req: &Request| -> bool { !req.headers.contains_key("authorization") };

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(no_auth_header)
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "choices": [{ "message": { "content": "no auth ok", "role": "assistant" } }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let endpoint = format!("{}/v1/chat/completions", server.uri());
    let runner = HttpRunner::new(endpoint);

    let out = runner
        .run(&user_prompt("hi"))
        .expect("expected Ok — no Authorization header should be sent");
    assert_eq!(out.stdout, "no auth ok");
}

/// 4. A 4xx response from the server must surface as `Err`. curl's
///    `--fail-with-body` exits non-zero on 4xx; the runner then runs
///    `is_auth_failure()` over the combined stdout/stderr. Either
///    `AuthFailed` or `NonZeroExit` is acceptable per the spec —
///    `is_auth_failure` keys on substrings like "401" / "authenticate"
///    / "invalid_api_key", and what curl exposes from a 401 body in
///    --fail-with-body mode is version-sensitive.
#[tokio::test(flavor = "current_thread")]
async fn http_runner_propagates_4xx_as_nonzero_or_authfailed() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
            "error": "Invalid API key"
        })))
        .mount(&server)
        .await;

    let endpoint = format!("{}/v1/chat/completions", server.uri());
    let runner = HttpRunner::new(endpoint);

    let err = runner
        .run(&user_prompt("hi"))
        .expect_err("expected Err on 401 response");

    assert!(
        matches!(
            err,
            RunnerError::AuthFailed(_) | RunnerError::NonZeroExit { .. }
        ),
        "expected AuthFailed or NonZeroExit, got: {err:?}"
    );
}

/// 5. With `prompt.system = None`, the messages array must contain
///    exactly one entry of role "user".
#[tokio::test(flavor = "current_thread")]
async fn http_runner_omits_system_message_when_none() {
    let server = MockServer::start().await;

    let body_matcher = |req: &Request| -> bool {
        let body: Value = match req.body_json() {
            Ok(v) => v,
            Err(_) => return false,
        };
        let messages = match body.get("messages").and_then(|m| m.as_array()) {
            Some(arr) => arr,
            None => return false,
        };
        messages.len() == 1 && messages[0].get("role").and_then(|r| r.as_str()) == Some("user")
    };

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_matcher)
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "choices": [{ "message": { "content": "ok", "role": "assistant" } }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let endpoint = format!("{}/v1/chat/completions", server.uri());
    let runner = HttpRunner::new(endpoint);

    let prompt = Prompt {
        system: None,
        user: "hello".into(),
        model: Some("test-model".into()),
        ..Default::default()
    };
    runner.run(&prompt).expect("expected Ok");
}

/// 6. With `prompt.system = Some(...)`, messages must be exactly
///    `[system, user]` in that order with the right contents.
#[tokio::test(flavor = "current_thread")]
async fn http_runner_includes_system_message_when_some() {
    let server = MockServer::start().await;

    let body_matcher = |req: &Request| -> bool {
        let body: Value = match req.body_json() {
            Ok(v) => v,
            Err(_) => return false,
        };
        let messages = match body.get("messages").and_then(|m| m.as_array()) {
            Some(arr) => arr,
            None => return false,
        };
        if messages.len() != 2 {
            return false;
        }
        let m0_role = messages[0].get("role").and_then(|r| r.as_str());
        let m0_content = messages[0].get("content").and_then(|c| c.as_str());
        let m1_role = messages[1].get("role").and_then(|r| r.as_str());
        m0_role == Some("system")
            && m0_content == Some("you are a helpful assistant")
            && m1_role == Some("user")
    };

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_matcher)
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "choices": [{ "message": { "content": "ok", "role": "assistant" } }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let endpoint = format!("{}/v1/chat/completions", server.uri());
    let runner = HttpRunner::new(endpoint);

    let prompt = Prompt {
        system: Some("you are a helpful assistant".into()),
        user: "hello".into(),
        model: Some("test-model".into()),
        ..Default::default()
    };
    runner.run(&prompt).expect("expected Ok");
}
