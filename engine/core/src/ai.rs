//! Choosing and managing the local models: which one answers, how it is run, and what is installed.

use rusqlite::Connection;
use serde::Serialize;
use serde_json::{json, Value};

use crate::embeddings::Embedder;
use crate::llm::{Flow, Llm, ModelInfo, PullProgress};
use crate::settings::{self, AppSettings};
use crate::{Error, ModelProblem, Result};

/// A modest context window: a model's own default can claim far more memory than teaching needs.
const NUM_CTX: i64 = 8192;
const MAX_ANSWER_TOKENS: i64 = 900;
const MAX_MODEL_NAME_CHARS: usize = 200;

/// How the model is run for every request the app makes.
pub fn inference_options(settings: &AppSettings) -> Value {
    let mut options = json!({ "num_ctx": NUM_CTX, "temperature": 0.2, "num_predict": MAX_ANSWER_TOKENS });
    if settings.compute_preference == "cpu" {
        options["num_gpu"] = json!(0);
    }
    options
}

/// The model that answers: the one chosen in Settings, else the first local one. Never a cloud model.
pub fn resolve_chat_model(models: &[ModelInfo], configured: Option<&str>) -> Result<String> {
    let mut local = models.iter().filter(|model| !model.is_cloud && model.can_chat());
    match configured.filter(|name| !name.is_empty()) {
        Some(name) if local.any(|model| model.name == name) => Ok(name.to_string()),
        Some(name) => Err(Error::Model(ModelProblem::NotInstalled, format!("The selected model {name} is not installed locally."))),
        None => local.next().map(|model| model.name.clone()).ok_or_else(|| Error::Model(ModelProblem::NotInstalled, "No local language model is installed. Add one in Settings.".into())),
    }
}

/// A real embedding model when one is chosen and installed, else the hashing fallback.
pub fn resolve_embedder<'a>(llm: &'a dyn Llm, models: &[ModelInfo], configured: Option<&str>) -> Embedder<'a> {
    match configured {
        Some(name) if models.iter().any(|model| model.name == name && model.can_embed() && !model.is_cloud) => Embedder::Model { llm, model: name.to_string() },
        _ => Embedder::Hashing,
    }
}

/// The chat model and the options for a request, from the saved settings and what is installed.
pub fn chat_setup(conn: &Connection, llm: &dyn Llm) -> Result<(String, Value)> {
    let settings = settings::load(conn)?;
    let model = resolve_chat_model(&llm.list_models()?, settings.llm_model.as_deref())?;
    Ok((model, inference_options(&settings)))
}

#[derive(Debug, Serialize)]
pub struct ModelView {
    pub name: String,
    pub size_bytes: i64,
    pub family: Option<String>,
    pub parameter_size: Option<String>,
    pub quantization: Option<String>,
    pub can_chat: bool,
    pub can_embed: bool,
}

#[derive(Debug, Serialize)]
pub struct RunningModel {
    pub name: String,
    /// cpu, gpu or mixed: where the loaded model sits.
    pub processor: &'static str,
    pub vram_bytes: i64,
    pub size_bytes: i64,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct EmbeddingStatus {
    /// "model" or "hashing".
    pub mode: &'static str,
    pub model: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Status {
    pub reachable: bool,
    pub version: Option<String>,
    /// What is wrong, in words for the learner, when something is.
    pub message: Option<String>,
    pub active_model: Option<String>,
    pub models: Vec<ModelView>,
    pub excluded_cloud_models: Vec<String>,
    pub running: Vec<RunningModel>,
    pub embedding: EmbeddingStatus,
}

/// What the runtime has and which models the app would use. Never fails: a runtime that cannot be
/// reached is itself the answer.
pub fn status(conn: &Connection, llm: &dyn Llm) -> Result<Status> {
    let settings = settings::load(conn)?;
    let reached = llm.version().and_then(|version| Ok((version, llm.list_models()?, llm.running()?)));
    let (version, models, running) = match reached {
        Ok(reached) => reached,
        Err(error) => {
            return Ok(Status {
                reachable: false, version: None, message: Some(error.to_string()), active_model: None, models: Vec::new(),
                excluded_cloud_models: Vec::new(), running: Vec::new(), embedding: EmbeddingStatus { mode: "hashing", model: None },
            })
        }
    };
    let (active_model, message) = match resolve_chat_model(&models, settings.llm_model.as_deref()) {
        Ok(model) => (Some(model), None),
        Err(error) => (None, Some(error.to_string())),
    };
    let embedding = match resolve_embedder(llm, &models, settings.embedding_model.as_deref()) {
        Embedder::Model { model, .. } => EmbeddingStatus { mode: "model", model: Some(model) },
        Embedder::Hashing => EmbeddingStatus { mode: "hashing", model: None },
    };
    Ok(Status {
        reachable: true,
        version: Some(version),
        message,
        active_model,
        excluded_cloud_models: models.iter().filter(|model| model.is_cloud).map(|model| model.name.clone()).collect(),
        models: models.into_iter().filter(|model| !model.is_cloud).map(|model| ModelView {
            can_chat: model.can_chat(), can_embed: model.can_embed(), name: model.name, size_bytes: model.size_bytes,
            family: model.family, parameter_size: model.parameter_size, quantization: model.quantization,
        }).collect(),
        running: running.into_iter().map(|entry| RunningModel {
            processor: if entry.size_vram == 0 { "cpu" } else if entry.size_vram >= entry.size { "gpu" } else { "mixed" },
            name: entry.name, vram_bytes: entry.size_vram, size_bytes: entry.size,
        }).collect(),
        embedding,
    })
}

fn checked_name(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_MODEL_NAME_CHARS {
        return Err(Error::Invalid("Give the name of a model, such as qwen3:4b.".into()));
    }
    Ok(name)
}

/// Downloads a model, reporting each step as an event. The last event is "done" or "error".
pub fn pull_model(llm: &dyn Llm, name: &str, emit: &mut dyn FnMut(Value) -> Flow) -> Result<()> {
    let name = checked_name(name)?;
    let mut stopped = false;
    let pulled = llm.pull(name, &mut |progress: PullProgress| {
        let flow = emit(json!({ "type": "progress", "status": progress.status, "total": progress.total, "completed": progress.completed }));
        stopped |= flow == Flow::Stop;
        flow
    });
    match pulled {
        Ok(()) if stopped => {}
        Ok(()) => {
            emit(json!({ "type": "done" }));
        }
        Err(error) => {
            emit(json!({ "type": "error", "message": error.to_string() }));
        }
    }
    Ok(())
}

pub fn delete_model(llm: &dyn Llm, name: &str) -> Result<()> {
    llm.delete(checked_name(name)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::testing::{model, FakeLlm};
    use crate::testing::library;

    fn problem<T: std::fmt::Debug>(result: Result<T>) -> ModelProblem {
        match result {
            Err(Error::Model(problem, _)) => problem,
            other => panic!("expected a model problem, got {other:?}"),
        }
    }

    #[test]
    fn a_cloud_model_is_never_chosen() {
        let cloud = model("remote:1", &["completion"], true);
        let local = model("local:1", &["completion"], false);
        let embedding_only = model("embed:1", &["embedding"], false);
        assert_eq!(resolve_chat_model(&[cloud.clone(), embedding_only.clone(), local.clone()], None).unwrap(), "local:1");
        assert_eq!(resolve_chat_model(&[cloud.clone(), local.clone()], Some("local:1")).unwrap(), "local:1");
        assert_eq!(problem(resolve_chat_model(&[cloud.clone(), local], Some("remote:1"))), ModelProblem::NotInstalled);
        assert_eq!(resolve_chat_model(&[cloud.clone()], None).unwrap_err().to_string(), "No local language model is installed. Add one in Settings.");
        assert_eq!(problem(resolve_chat_model(&[embedding_only], Some("embed:1"))), ModelProblem::NotInstalled);
    }

    #[test]
    fn preferring_the_cpu_keeps_the_model_off_the_gpu() {
        let on_cpu = inference_options(&AppSettings { compute_preference: "cpu".into(), ..AppSettings::default() });
        assert_eq!(on_cpu, json!({ "num_ctx": 8192, "temperature": 0.2, "num_predict": 900, "num_gpu": 0 }));
        assert!(inference_options(&AppSettings::default()).get("num_gpu").is_none());
    }

    #[test]
    fn status_lists_local_models_and_names_the_cloud_ones_left_out() {
        let conn = library();
        let fake = FakeLlm::new().with_models(vec![model("local:1", &["completion"], false), model("remote:1", &["completion"], true), model("embed:1", &["embedding"], false)]);
        let status = status(&conn, &fake).unwrap();
        assert_eq!((status.reachable, status.version.as_deref(), status.active_model.as_deref(), &status.message), (true, Some("9.9.9"), Some("local:1"), &None));
        assert_eq!(status.models.iter().map(|model| (model.name.as_str(), model.can_chat, model.can_embed)).collect::<Vec<_>>(), [("local:1", true, false), ("embed:1", false, true)]);
        assert_eq!(status.excluded_cloud_models, ["remote:1"]);
        assert_eq!((status.running[0].processor, status.running[0].size_bytes), ("cpu", 100));
        assert_eq!(status.embedding, EmbeddingStatus { mode: "hashing", model: None });
    }

    #[test]
    fn status_follows_the_models_chosen_in_settings() {
        let conn = library();
        let fake = FakeLlm::new().with_models(vec![model("local:1", &["completion"], false), model("embed:1", &["embedding"], false)]);
        settings::save(&conn, AppSettings { embedding_model: Some("embed:1".into()), llm_model: Some("gone:1".into()), ..AppSettings::default() }).unwrap();
        let status = status(&conn, &fake).unwrap();
        assert_eq!(status.embedding, EmbeddingStatus { mode: "model", model: Some("embed:1".into()) });
        // A chosen model that is no longer installed is said so, and nothing is quietly used instead.
        assert_eq!((status.active_model, status.message.as_deref()), (None, Some("The selected model gone:1 is not installed locally.")));
    }

    #[test]
    fn status_says_when_the_runtime_is_not_running() {
        let fake = FakeLlm::new();
        fake.go_down();
        let status = status(&library(), &fake).unwrap();
        assert!(!status.reachable && status.models.is_empty() && status.message.unwrap().contains("not running"));
    }

    #[test]
    fn a_download_reports_progress_then_that_it_is_done() {
        let fake = FakeLlm::new();
        let mut events = Vec::new();
        pull_model(&fake, " new:1 ", &mut |event| {
            events.push(event);
            Flow::Continue
        }).unwrap();
        assert_eq!(*fake.pulled.lock().unwrap(), ["new:1"]);
        assert_eq!(events[0], json!({ "type": "progress", "status": "pulling", "total": 100, "completed": 50 }));
        assert_eq!(events.last().unwrap(), &json!({ "type": "done" }));
    }

    #[test]
    fn a_download_that_is_stopped_or_fails_does_not_claim_to_be_done() {
        let fake = FakeLlm::new();
        let mut events = Vec::new();
        pull_model(&fake, "new:1", &mut |event| {
            events.push(event);
            Flow::Stop
        }).unwrap();
        assert_eq!(events.len(), 1);
        fake.go_down();
        let mut events = Vec::new();
        pull_model(&fake, "new:1", &mut |event| {
            events.push(event);
            Flow::Continue
        }).unwrap();
        assert_eq!(events, [json!({ "type": "error", "message": "Ollama is not running. Start Ollama and try again." })]);
        assert!(matches!(pull_model(&fake, "  ", &mut |_| Flow::Continue), Err(Error::Invalid(_))));
    }

    #[test]
    fn a_model_is_removed_by_name() {
        let fake = FakeLlm::new();
        delete_model(&fake, "old:1").unwrap();
        assert_eq!(*fake.deleted.lock().unwrap(), ["old:1"]);
        assert!(delete_model(&fake, "").is_err());
    }
}
