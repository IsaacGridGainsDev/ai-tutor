//! Talking to the local model runtime. `Llm` is everything the engines need from it, so the
//! runtime stays interchangeable; `Ollama` is the one implementation.

use std::io::{BufRead, BufReader};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{Error, ModelProblem, Result};

pub const DEFAULT_URL: &str = "http://127.0.0.1:11434";
const KEEP_ALIVE: &str = "10m";
const NOT_RUNNING: &str = "Ollama is not running. Start Ollama and try again.";

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ModelInfo {
    pub name: String,
    pub size_bytes: i64,
    pub family: Option<String>,
    pub parameter_size: Option<String>,
    pub quantization: Option<String>,
    pub capabilities: Vec<String>,
    /// Listed locally, but prompts would be sent to another computer.
    pub is_cloud: bool,
}

impl ModelInfo {
    pub fn can_chat(&self) -> bool {
        self.capabilities.iter().any(|capability| capability == "completion")
    }

    pub fn can_embed(&self) -> bool {
        self.capabilities.iter().any(|capability| capability == "embedding")
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Message {
    pub role: &'static str,
    pub content: String,
}

impl Message {
    pub fn system(content: &str) -> Self {
        Self { role: "system", content: content.to_string() }
    }

    pub fn user(content: String) -> Self {
        Self { role: "user", content }
    }
}

/// One piece of a reply as it arrives. The last piece carries the counts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Chunk {
    pub delta: String,
    pub done: bool,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub generation_seconds: f64,
}

/// A model loaded in memory.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Running {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub size: i64,
    #[serde(default)]
    pub size_vram: i64,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct PullProgress {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub total: i64,
    #[serde(default)]
    pub completed: i64,
}

/// What a listener wants after each piece: more, or for the work to stop.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Flow {
    Continue,
    Stop,
}

pub trait Llm: Send + Sync {
    fn version(&self) -> Result<String>;
    fn list_models(&self) -> Result<Vec<ModelInfo>>;
    fn running(&self) -> Result<Vec<Running>>;
    /// Streams a reply. `schema`, when given, is the JSON shape the reply must have.
    /// Returning `Flow::Stop` from the listener ends generation.
    fn chat(&self, model: &str, messages: &[Message], schema: Option<&Value>, options: &Value, on_chunk: &mut dyn FnMut(Chunk) -> Flow) -> Result<()>;
    fn embed(&self, model: &str, texts: &[String]) -> Result<Vec<Vec<f32>>>;
    fn pull(&self, model: &str, on_progress: &mut dyn FnMut(PullProgress) -> Flow) -> Result<()>;
    fn delete(&self, model: &str) -> Result<()>;
}

/// The whole reply as one string, for callers that do not show progress.
pub fn complete(llm: &dyn Llm, model: &str, messages: &[Message], schema: &Value, options: &Value) -> Result<String> {
    let mut content = String::new();
    llm.chat(model, messages, Some(schema), options, &mut |chunk| {
        content.push_str(&chunk.delta);
        Flow::Continue
    })?;
    Ok(content)
}

pub fn is_cloud_name(name: &str) -> bool {
    name.ends_with(":cloud") || name.ends_with("-cloud")
}

/// Cloud models appear in the local list but forward prompts to a remote host.
fn is_cloud_entry(entry: &Value) -> bool {
    let filled = |key: &str| entry.get(key).is_some_and(|value| !value.is_null() && value != "");
    filled("remote_host") || filled("remote_model") || is_cloud_name(entry["name"].as_str().unwrap_or(""))
}

fn cloud_refused(model: &str) -> Error {
    Error::Model(ModelProblem::Cloud, format!("{model} is a cloud model. Documents must stay on this computer."))
}

fn not_running() -> Error {
    Error::Model(ModelProblem::Unavailable, NOT_RUNNING.into())
}

fn failed(message: String) -> Error {
    Error::Model(ModelProblem::Failed, message)
}

pub struct Ollama {
    base_url: String,
    agent: ureq::Agent,
}

impl Ollama {
    pub fn new(base_url: &str) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(3))
            // A model on the CPU can take minutes before its first word.
            .timeout_read(Duration::from_secs(900))
            .timeout_write(Duration::from_secs(30))
            .build();
        Self { base_url: base_url.trim_end_matches('/').to_string(), agent }
    }

    /// The runtime's address: `TUTOR_OLLAMA_URL` when set, else the usual local one.
    pub fn from_environment() -> Self {
        Self::new(&std::env::var("TUTOR_OLLAMA_URL").unwrap_or_else(|_| DEFAULT_URL.to_string()))
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    /// Turns a refusal from the runtime into words for the learner.
    fn refusal(error: ureq::Error) -> Error {
        match error {
            ureq::Error::Status(code, response) => {
                let body = response.into_string().unwrap_or_default();
                let text = serde_json::from_str::<Value>(&body).ok().and_then(|value| value["error"].as_str().map(str::to_string)).filter(|text| !text.is_empty()).unwrap_or(body);
                if code == 404 {
                    Error::Model(ModelProblem::NotInstalled, text)
                } else {
                    failed(format!("Ollama error: {text}"))
                }
            }
            ureq::Error::Transport(_) => not_running(),
        }
    }

    fn get(&self, path: &str) -> Result<Value> {
        let response = self.agent.get(&self.url(path)).call().map_err(Self::refusal)?;
        response.into_json().map_err(|error| failed(format!("Ollama error: {error}")))
    }

    /// Checked before every call that sends text, so no setting can send a book off this computer.
    fn require_local(&self, model: &str) -> Result<ModelInfo> {
        if is_cloud_name(model) {
            return Err(cloud_refused(model));
        }
        match self.list_models()?.into_iter().find(|info| info.name == model) {
            Some(info) if info.is_cloud => Err(cloud_refused(model)),
            Some(info) => Ok(info),
            None => Err(Error::Model(ModelProblem::NotInstalled, format!("The model {model} is not installed."))),
        }
    }

    /// Posts a request whose answer arrives as one JSON object per line.
    fn stream(&self, path: &str, payload: &Value, failure: &str, on_line: &mut dyn FnMut(Value) -> Flow) -> Result<()> {
        let response = self.agent.post(&self.url(path)).send_json(payload).map_err(Self::refusal)?;
        for line in BufReader::new(response.into_reader()).lines() {
            let line = line.map_err(|_| not_running())?;
            if line.trim().is_empty() {
                continue;
            }
            let event: Value = serde_json::from_str(&line).map_err(|_| failed(format!("{failure}: the reply could not be read.")))?;
            if let Some(error) = event.get("error") {
                return Err(failed(format!("{failure}: {}", error.as_str().map(str::to_string).unwrap_or_else(|| error.to_string()))));
            }
            if on_line(event) == Flow::Stop {
                // Leaving here closes the connection, which is what stops the runtime generating.
                break;
            }
        }
        Ok(())
    }
}

fn text_field(value: &Value) -> Option<String> {
    value.as_str().filter(|text| !text.is_empty()).map(str::to_string)
}

impl Llm for Ollama {
    fn version(&self) -> Result<String> {
        Ok(self.get("/api/version")?["version"].as_str().unwrap_or("").to_string())
    }

    fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let listed = self.get("/api/tags")?;
        let entries = listed["models"].as_array().cloned().unwrap_or_default();
        Ok(entries.iter().filter_map(|entry| {
            let details = &entry["details"];
            Some(ModelInfo {
                name: entry["name"].as_str()?.to_string(),
                size_bytes: entry["size"].as_i64().unwrap_or(0),
                family: text_field(&details["family"]),
                parameter_size: text_field(&details["parameter_size"]),
                quantization: text_field(&details["quantization_level"]),
                capabilities: entry["capabilities"].as_array().map(|all| all.iter().filter_map(|one| one.as_str().map(str::to_string)).collect()).unwrap_or_default(),
                is_cloud: is_cloud_entry(entry),
            })
        }).collect())
    }

    fn running(&self) -> Result<Vec<Running>> {
        let loaded = self.get("/api/ps")?;
        Ok(serde_json::from_value(loaded["models"].clone()).unwrap_or_default())
    }

    fn chat(&self, model: &str, messages: &[Message], schema: Option<&Value>, options: &Value, on_chunk: &mut dyn FnMut(Chunk) -> Flow) -> Result<()> {
        let info = self.require_local(model)?;
        let mut payload = json!({ "model": model, "messages": messages, "stream": true, "keep_alive": KEEP_ALIVE, "options": options });
        if let Some(schema) = schema {
            payload["format"] = schema.clone();
        }
        // Thinking aloud first would add minutes on a CPU before any answer.
        if info.capabilities.iter().any(|capability| capability == "thinking") {
            payload["think"] = Value::Bool(false);
        }
        self.stream("/api/chat", &payload, "Ollama error", &mut |event| {
            on_chunk(Chunk {
                delta: event["message"]["content"].as_str().unwrap_or("").to_string(),
                done: event["done"].as_bool().unwrap_or(false),
                prompt_tokens: event["prompt_eval_count"].as_i64().unwrap_or(0),
                completion_tokens: event["eval_count"].as_i64().unwrap_or(0),
                generation_seconds: event["eval_duration"].as_f64().unwrap_or(0.0) / 1e9,
            })
        })
    }

    fn embed(&self, model: &str, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.require_local(model)?;
        let payload = json!({ "model": model, "input": texts, "keep_alive": KEEP_ALIVE });
        let response = self.agent.post(&self.url("/api/embed")).send_json(payload).map_err(Self::refusal)?;
        let body: Value = response.into_json().map_err(|error| failed(format!("Ollama error: {error}")))?;
        serde_json::from_value(body["embeddings"].clone()).map_err(|_| failed("Ollama error: the embeddings could not be read.".into()))
    }

    fn pull(&self, model: &str, on_progress: &mut dyn FnMut(PullProgress) -> Flow) -> Result<()> {
        if is_cloud_name(model) {
            return Err(cloud_refused(model));
        }
        self.stream("/api/pull", &json!({ "model": model, "stream": true }), "Download failed", &mut |event| {
            on_progress(serde_json::from_value(event).unwrap_or_default())
        })
    }

    fn delete(&self, model: &str) -> Result<()> {
        match self.agent.request("DELETE", &self.url("/api/delete")).send_json(json!({ "model": model })) {
            Ok(_) => Ok(()),
            Err(ureq::Error::Status(404, _)) => Err(Error::Model(ModelProblem::NotInstalled, format!("The model {model} is not installed."))),
            Err(error) => Err(Self::refusal(error)),
        }
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex;

    use super::*;

    pub fn model(name: &str, capabilities: &[&str], is_cloud: bool) -> ModelInfo {
        ModelInfo {
            name: name.into(),
            size_bytes: 1000,
            family: Some("fam".into()),
            parameter_size: Some("1B".into()),
            quantization: Some("Q4".into()),
            capabilities: capabilities.iter().map(|capability| capability.to_string()).collect(),
            is_cloud,
        }
    }

    pub struct Call {
        pub model: String,
        pub messages: Vec<Message>,
        pub options: Value,
    }

    /// A scripted runtime that records what the engines send to it.
    pub struct FakeLlm {
        /// What the model "says", sent a few characters at a time.
        pub reply: Mutex<String>,
        pub models: Mutex<Vec<ModelInfo>>,
        pub calls: Mutex<Vec<Call>>,
        pub down: AtomicBool,
        /// Replies that were stopped before their end.
        pub stopped: AtomicUsize,
        pub pulled: Mutex<Vec<String>>,
        pub deleted: Mutex<Vec<String>>,
    }

    impl FakeLlm {
        pub fn new() -> Self {
            Self {
                reply: Mutex::new(r#"{"segments": []}"#.into()),
                models: Mutex::new(vec![model("local:1", &["completion"], false)]),
                calls: Mutex::new(Vec::new()),
                down: AtomicBool::new(false),
                stopped: AtomicUsize::new(0),
                pulled: Mutex::new(Vec::new()),
                deleted: Mutex::new(Vec::new()),
            }
        }

        pub fn replying(reply: Value) -> Self {
            let fake = Self::new();
            fake.say(reply);
            fake
        }

        pub fn say(&self, reply: Value) {
            *self.reply.lock().unwrap() = match reply {
                Value::String(text) => text,
                other => other.to_string(),
            };
        }

        pub fn with_models(self, models: Vec<ModelInfo>) -> Self {
            *self.models.lock().unwrap() = models;
            self
        }

        pub fn go_down(&self) {
            self.down.store(true, Ordering::SeqCst);
        }

        pub fn call_count(&self) -> usize {
            self.calls.lock().unwrap().len()
        }

        /// The text of the learner's turn in the given call.
        pub fn prompt(&self, call: usize) -> String {
            self.calls.lock().unwrap()[call].messages[1].content.clone()
        }

        fn check(&self) -> Result<()> {
            if self.down.load(Ordering::SeqCst) { Err(not_running()) } else { Ok(()) }
        }
    }

    impl Llm for FakeLlm {
        fn version(&self) -> Result<String> {
            self.check()?;
            Ok("9.9.9".into())
        }

        fn list_models(&self) -> Result<Vec<ModelInfo>> {
            self.check()?;
            Ok(self.models.lock().unwrap().clone())
        }

        fn running(&self) -> Result<Vec<Running>> {
            self.check()?;
            Ok(vec![Running { name: "local:1".into(), size: 100, size_vram: 0 }])
        }

        fn chat(&self, model: &str, messages: &[Message], _schema: Option<&Value>, options: &Value, on_chunk: &mut dyn FnMut(Chunk) -> Flow) -> Result<()> {
            self.check()?;
            self.calls.lock().unwrap().push(Call { model: model.into(), messages: messages.to_vec(), options: options.clone() });
            let reply: Vec<char> = self.reply.lock().unwrap().chars().collect();
            for piece in reply.chunks(8) {
                if on_chunk(Chunk { delta: piece.iter().collect(), ..Chunk::default() }) == Flow::Stop {
                    self.stopped.fetch_add(1, Ordering::SeqCst);
                    return Ok(());
                }
            }
            on_chunk(Chunk { done: true, prompt_tokens: 11, completion_tokens: 22, generation_seconds: 1.5, ..Chunk::default() });
            Ok(())
        }

        fn embed(&self, _model: &str, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            self.check()?;
            Ok(texts.iter().map(|text| vec![text.chars().count() as f32, 1.0, 0.5]).collect())
        }

        fn pull(&self, model: &str, on_progress: &mut dyn FnMut(PullProgress) -> Flow) -> Result<()> {
            self.check()?;
            self.pulled.lock().unwrap().push(model.into());
            if on_progress(PullProgress { status: "pulling".into(), total: 100, completed: 50 }) == Flow::Stop {
                self.stopped.fetch_add(1, Ordering::SeqCst);
                return Ok(());
            }
            on_progress(PullProgress { status: "success".into(), ..PullProgress::default() });
            Ok(())
        }

        fn delete(&self, model: &str) -> Result<()> {
            self.check()?;
            self.deleted.lock().unwrap().push(model.into());
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    use super::*;

    type Seen = Arc<Mutex<Vec<(String, Value)>>>;

    /// A stand-in runtime on a local port. It lists one local and one cloud model, and answers
    /// everything else with what `respond` returns for the path: a status and a body.
    fn runtime(respond: impl Fn(&str) -> (u16, String) + Send + 'static) -> (Ollama, Seen) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let seen: Seen = Arc::default();
        let record = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request_line = String::new();
                if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
                    continue;
                }
                let path = request_line.split_whitespace().nth(1).unwrap_or("").to_string();
                let mut length = 0;
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    if header.trim().is_empty() {
                        break;
                    }
                    if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap();
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                record.lock().unwrap().push((path.clone(), serde_json::from_slice(&body).unwrap_or(Value::Null)));
                let (status, body) = if path == "/api/tags" {
                    (200, json!({ "models": [
                        { "name": "local:1", "size": 9000, "details": { "family": "f" }, "capabilities": ["completion", "thinking"] },
                        { "name": "remote:latest", "size": 300, "remote_host": "https://ollama.com", "details": {}, "capabilities": ["completion"] },
                    ] }).to_string())
                } else {
                    respond(&path)
                };
                let _ = write!(stream, "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        (Ollama::new(&format!("http://{address}")), seen)
    }

    fn problem<T: std::fmt::Debug>(result: Result<T>) -> (ModelProblem, String) {
        match result {
            Err(Error::Model(problem, message)) => (problem, message),
            other => panic!("expected a model problem, got {other:?}"),
        }
    }

    fn paths(seen: &Seen) -> Vec<String> {
        seen.lock().unwrap().iter().map(|(path, _)| path.clone()).collect()
    }

    fn chat(ollama: &Ollama, model: &str) -> Result<Vec<Chunk>> {
        let mut chunks = Vec::new();
        ollama.chat(model, &[Message::user("private book text".into())], None, &json!({ "num_ctx": 8192 }), &mut |chunk| {
            chunks.push(chunk);
            Flow::Continue
        })?;
        Ok(chunks)
    }

    #[test]
    fn cloud_models_are_flagged_by_where_they_run() {
        let (ollama, _) = runtime(|_| (500, String::new()));
        let flags: Vec<(String, bool)> = ollama.list_models().unwrap().into_iter().map(|model| (model.name, model.is_cloud)).collect();
        assert_eq!(flags, [("local:1".to_string(), false), ("remote:latest".to_string(), true)]);
        let local = &ollama.list_models().unwrap()[0];
        assert_eq!((local.size_bytes, local.family.as_deref(), local.parameter_size.as_deref(), local.can_chat(), local.can_embed()), (9000, Some("f"), None, true, false));
    }

    #[test]
    fn a_cloud_model_is_refused_even_without_cloud_in_its_name() {
        let (ollama, seen) = runtime(|_| (200, "{}".into()));
        assert_eq!(problem(chat(&ollama, "remote:latest")).0, ModelProblem::Cloud);
        assert_eq!(problem(ollama.embed("remote:latest", &["private".into()])).0, ModelProblem::Cloud);
        // The book's text never left: only the list of models was asked for.
        assert!(paths(&seen).iter().all(|path| path == "/api/tags"));
    }

    #[test]
    fn a_cloud_name_is_refused_for_chat_and_download_without_asking_the_runtime() {
        let (ollama, seen) = runtime(|_| (200, "{}".into()));
        assert_eq!(problem(chat(&ollama, "x:cloud")).0, ModelProblem::Cloud);
        assert_eq!(problem(ollama.pull("x-cloud", &mut |_| Flow::Continue)), (ModelProblem::Cloud, "x-cloud is a cloud model. Documents must stay on this computer.".to_string()));
        assert!(paths(&seen).is_empty());
    }

    #[test]
    fn a_model_that_is_not_installed_is_reported() {
        let (ollama, _) = runtime(|_| (404, r#"{"error": "model not found"}"#.into()));
        assert_eq!(problem(chat(&ollama, "absent:1")), (ModelProblem::NotInstalled, "The model absent:1 is not installed.".to_string()));
        assert_eq!(problem(ollama.delete("absent:1")), (ModelProblem::NotInstalled, "The model absent:1 is not installed.".to_string()));
    }

    #[test]
    fn a_runtime_that_is_not_running_is_reported() {
        // A port nothing listens on: bound, read, and released.
        let free = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
        let ollama = Ollama::new(&format!("http://{free}"));
        assert_eq!(problem(ollama.list_models()), (ModelProblem::Unavailable, NOT_RUNNING.to_string()));
        assert_eq!(problem(ollama.version()).0, ModelProblem::Unavailable);
    }

    #[test]
    fn a_reply_arrives_in_pieces_with_thinking_turned_off() {
        let (ollama, seen) = runtime(|_| (200, [
            json!({ "message": { "content": "Hel" }, "done": false }),
            json!({ "message": { "content": "lo" }, "done": false }),
            json!({ "message": { "content": "" }, "done": true, "prompt_eval_count": 7, "eval_count": 2, "eval_duration": 2_000_000_000u64 }),
        ].map(|line| line.to_string()).join("\n")));
        let chunks = chat(&ollama, "local:1").unwrap();
        assert_eq!(chunks.iter().map(|chunk| chunk.delta.as_str()).collect::<String>(), "Hello");
        let last = chunks.last().unwrap();
        assert_eq!((last.done, last.prompt_tokens, last.completion_tokens, last.generation_seconds), (true, 7, 2, 2.0));
        let sent = seen.lock().unwrap();
        let (path, body) = sent.last().unwrap();
        assert_eq!(path, "/api/chat");
        assert_eq!((&body["think"], &body["options"], &body["stream"], &body["model"]), (&json!(false), &json!({ "num_ctx": 8192 }), &json!(true), &json!("local:1")));
        assert_eq!(body["messages"], json!([{ "role": "user", "content": "private book text" }]));
        assert!(body.get("format").is_none());
    }

    #[test]
    fn a_listener_can_stop_a_reply() {
        let (ollama, _) = runtime(|_| (200, (0..50).map(|n| json!({ "message": { "content": n.to_string() } }).to_string()).collect::<Vec<_>>().join("\n")));
        let mut received = 0;
        ollama.chat("local:1", &[], Some(&json!({ "type": "object" })), &json!({}), &mut |_| {
            received += 1;
            if received == 3 { Flow::Stop } else { Flow::Continue }
        }).unwrap();
        assert_eq!(received, 3);
    }

    #[test]
    fn an_error_in_the_middle_of_a_reply_is_reported() {
        let (ollama, _) = runtime(|_| (200, r#"{"error": "out of memory"}"#.into()));
        assert_eq!(problem(chat(&ollama, "local:1")), (ModelProblem::Failed, "Ollama error: out of memory".to_string()));
        let (ollama, _) = runtime(|_| (500, r#"{"error": "it broke"}"#.into()));
        assert_eq!(problem(chat(&ollama, "local:1")).1, "Ollama error: it broke");
    }

    #[test]
    fn downloads_report_progress_and_failures() {
        let (ollama, seen) = runtime(|_| (200, "{\"status\": \"pulling\", \"total\": 100, \"completed\": 50}\n\n{\"status\": \"success\"}\n".into()));
        let mut progress = Vec::new();
        ollama.pull("new:1", &mut |event| {
            progress.push(event);
            Flow::Continue
        }).unwrap();
        assert_eq!(progress, [PullProgress { status: "pulling".into(), total: 100, completed: 50 }, PullProgress { status: "success".into(), total: 0, completed: 0 }]);
        assert_eq!(seen.lock().unwrap()[0], ("/api/pull".to_string(), json!({ "model": "new:1", "stream": true })));
        let (ollama, _) = runtime(|_| (200, r#"{"error": "no such model"}"#.into()));
        assert_eq!(problem(ollama.pull("typo:1", &mut |_| Flow::Continue)).1, "Download failed: no such model");
    }

    #[test]
    fn embeddings_versions_loaded_models_and_removal_are_read() {
        let (ollama, seen) = runtime(|path| (200, match path {
            "/api/embed" => r#"{"embeddings": [[1.0, 0.5], [0.0, 2.0]]}"#,
            "/api/version" => r#"{"version": "0.12.3"}"#,
            "/api/ps" => r#"{"models": [{"name": "local:1", "size": 100, "size_vram": 40}]}"#,
            _ => "{}",
        }.to_string()));
        assert_eq!(ollama.embed("local:1", &["a".into(), "b".into()]).unwrap(), [[1.0, 0.5], [0.0, 2.0]]);
        assert_eq!(ollama.version().unwrap(), "0.12.3");
        let running = ollama.running().unwrap();
        assert_eq!((running[0].name.as_str(), running[0].size, running[0].size_vram), ("local:1", 100, 40));
        ollama.delete("local:1").unwrap();
        assert_eq!(seen.lock().unwrap().last().unwrap(), &("/api/delete".to_string(), json!({ "model": "local:1" })));
    }
}
