use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const ENDPOINT: &str = "https://api.typesafe.ai/v1/systemone";
/// Pinned, not `jev-latest`: the dead band is tuned against this version's
/// calibration. Decision clause 7.
pub const MODEL: &str = "jev-1.13.0";
pub const API_KEY_ENV: &str = "TYPESAFE_API_KEY";
/// Overrides `ENDPOINT`. Exists so the `--jev` path can be driven end to end
/// against a loopback stub without a key; unset in normal use.
pub const ENDPOINT_ENV: &str = "TYPESAFE_ENDPOINT";

/// The `state` field: the source group's paths and its staged diff, and
/// nothing else. Decision clause 8.
#[derive(Debug, Serialize)]
pub struct State {
    pub files: Vec<String>,
    pub diff: String,
}

#[derive(Debug, Serialize)]
pub struct Criteria {
    #[serde(rename = "true")]
    pub yes: &'static str,
    #[serde(rename = "false")]
    pub no: &'static str,
}

#[derive(Debug, Serialize)]
pub struct Noul {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub instructions: &'static str,
    pub criteria: Criteria,
}

#[derive(Debug, Serialize)]
pub struct Request<'a> {
    pub state: &'a State,
    pub model: &'a str,
    pub questions: BTreeMap<&'static str, Noul>,
}

#[derive(Debug, Deserialize)]
pub struct Answer {
    pub noul: f64,
}

#[derive(Debug, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Deserialize)]
pub struct Response {
    pub model: String,
    pub answers: BTreeMap<String, Answer>,
    pub usage: Usage,
}

pub trait Transport {
    /// POST `body` to the evaluation endpoint and return the response body.
    fn post(&self, body: &str) -> Result<String, String>;
}

/// Serializes the request, sends it, and parses the response.
pub fn ask(
    transport: &dyn Transport,
    state: &State,
    questions: BTreeMap<&'static str, Noul>,
) -> Result<Response, String> {
    let request = Request {
        state,
        model: MODEL,
        questions,
    };
    let body = serde_json::to_string(&request).map_err(|e| format!("jev request: {e}"))?;
    let raw = transport.post(&body)?;
    serde_json::from_str(&raw).map_err(|e| format!("jev response: {e}"))
}

/// The real transport. Reads the API key from the environment on construction
/// so a missing key fails before any work is done. `TYPESAFE_ENDPOINT`
/// overrides the endpoint, so the `--jev` path is provable end to end against
/// a loopback stub with no key.
pub struct Http {
    endpoint: String,
    key: String,
}

impl Http {
    pub fn from_env() -> Result<Self, String> {
        let key = std::env::var(API_KEY_ENV)
            .map_err(|_| format!("--jev requires {API_KEY_ENV} in the environment"))?;
        let endpoint = std::env::var(ENDPOINT_ENV).unwrap_or_else(|_| ENDPOINT.to_string());
        Ok(Self { endpoint, key })
    }
}

impl Transport for Http {
    fn post(&self, body: &str) -> Result<String, String> {
        ureq::post(&self.endpoint)
            .header("Authorization", &format!("Bearer {}", self.key))
            .header("Content-Type", "application/json")
            .send(body)
            .map_err(|e| format!("jev request failed: {e}"))?
            .body_mut()
            .read_to_string()
            .map_err(|e| format!("jev response failed: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use super::*;

    #[test]
    fn serializes_a_request_in_the_documented_shape() {
        let state = State {
            files: vec!["src/a.rs".into()],
            diff: "@@ -1 +1 @@\n-a\n+b\n".into(),
        };
        let mut questions = BTreeMap::new();
        questions.insert(
            "is_urgent",
            Noul {
                kind: "noul",
                instructions: "Does this convey urgency?",
                criteria: Criteria {
                    yes: "It does",
                    no: "It does not",
                },
            },
        );
        let body = serde_json::to_string(&Request {
            state: &state,
            model: MODEL,
            questions,
        })
        .expect("serialize");

        assert!(body.contains(r#""model":"jev-1.13.0""#));
        assert!(body.contains(r#""type":"noul""#));
        assert!(body.contains(r#""criteria":{"true":"It does","false":"It does not"}"#));
        assert!(body.contains(r#""files":["src/a.rs"]"#));
    }

    #[test]
    fn deserializes_a_documented_response() {
        let raw = r#"{
          "model": "jev-1.13.0",
          "answers": { "is_urgent": { "type": "noul", "noul": 0.95 } },
          "usage": { "input_tokens": 296, "output_tokens": 20 }
        }"#;
        let parsed: Response = serde_json::from_str(raw).expect("parse");

        assert_eq!(parsed.answers["is_urgent"].noul, 0.95);
        assert_eq!(parsed.usage.input_tokens, 296);
    }

    struct Canned(&'static str);

    impl Transport for Canned {
        fn post(&self, _body: &str) -> Result<String, String> {
            Ok(self.0.to_string())
        }
    }

    #[test]
    fn ask_parses_answers_from_the_transport() {
        let canned = Canned(
            r#"{"model":"jev-1.13.0",
                "answers":{"a":{"type":"noul","noul":0.9}},
                "usage":{"input_tokens":1,"output_tokens":2}}"#,
        );
        let state = State {
            files: vec!["src/a.rs".into()],
            diff: "@@".into(),
        };
        let response = ask(&canned, &state, BTreeMap::new()).expect("ask");

        assert_eq!(response.answers["a"].noul, 0.9);
    }

    #[test]
    fn ask_reports_unparseable_bodies() {
        struct Garbage;
        impl Transport for Garbage {
            fn post(&self, _body: &str) -> Result<String, String> {
                Ok("not json".into())
            }
        }
        let state = State {
            files: Vec::new(),
            diff: String::new(),
        };
        let err = ask(&Garbage, &state, BTreeMap::new()).expect_err("should fail");

        assert!(err.contains("jev response"), "unexpected error: {err}");
    }

    /// Accepts one connection, reads the request head and its body (via
    /// `Content-Length`), sends `response` verbatim, and returns the request
    /// head plus body as text.
    fn serve_once(response: &'static str) -> (String, String) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            let mut head = String::new();
            let mut content_length = 0usize;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).expect("read line");
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                if let Some(len) = line
                    .to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .map(str::trim)
                {
                    content_length = len.parse().unwrap_or(0);
                }
                head.push_str(&line);
            }
            let mut body = vec![0u8; content_length];
            reader.read_exact(&mut body).expect("read body");
            let mut stream = stream;
            stream.write_all(response.as_bytes()).expect("write");
            (head, String::from_utf8_lossy(&body).into_owned())
        });
        let endpoint = format!("http://{addr}/v1/systemone");
        let http = Http {
            endpoint,
            key: "test".into(),
        };
        let raw = http.post(r#"{"probe":true}"#).expect("post");
        let (head, captured_body) = handle.join().expect("server thread");
        assert!(raw.contains("noul"), "unexpected response: {raw}");
        (head, captured_body)
    }

    #[test]
    fn http_posts_with_bearer_auth_and_reads_the_body() {
        let response = "HTTP/1.1 200 OK\r\n\
             Content-Type: application/json\r\n\
             Content-Length: 88\r\n\
             Connection: close\r\n\r\n\
             {\"model\":\"jev-1.13.0\",\"answers\":{\"a\":{\"type\":\"noul\",\"noul\":0.5}},\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}";
        let (head, body) = serve_once(response);

        assert!(head.starts_with("POST /v1/systemone"), "{head}");
        assert!(
            head.to_ascii_lowercase()
                .contains("authorization: bearer test"),
            "{head}"
        );
        assert_eq!(body, r#"{"probe":true}"#);
    }

    #[test]
    fn http_surfaces_an_unauthorized_status_as_an_error() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            let mut line = String::new();
            loop {
                line.clear();
                reader.read_line(&mut line).expect("read line");
                if line == "\r\n" || line.is_empty() {
                    break;
                }
            }
            stream
                .write_all(
                    b"HTTP/1.1 401 Unauthorized\r\n\
                      Content-Length: 0\r\n\
                      Connection: close\r\n\r\n",
                )
                .expect("write");
        });
        let http = Http {
            endpoint: format!("http://{addr}/v1/systemone"),
            key: "bad".into(),
        };

        let err = http.post("{}").expect_err("should fail");

        assert!(err.contains("401"), "unexpected error: {err}");
    }
}
