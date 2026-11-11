//! The local model, served from the Rust engine: Ask, concepts, model-planned diagrams, and
//! managing which models are installed.
//!
//! Work that runs the model can take minutes, so it runs off the threads that serve the window,
//! reports progress over a channel, and can be stopped by its id.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tauri::ipc::Channel;
use tauri::State;
use tutor_core::llm::{Flow, Llm, Ollama};
use tutor_core::vectors::VectorStore;
use tutor_core::{ai, answer, concepts, diagram};

use crate::visual::{data_dir, with_library};

/// The model runtime, and the long-running requests that can still be stopped.
pub struct Models {
    llm: Arc<dyn Llm>,
    stops: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl Models {
    pub fn new() -> Self {
        Self { llm: Arc::new(Ollama::from_environment()), stops: Mutex::default() }
    }

    fn begin(&self, id: &str) -> Arc<AtomicBool> {
        let stop = Arc::new(AtomicBool::new(false));
        self.stops.lock().unwrap().insert(id.to_string(), stop.clone());
        stop
    }

    fn end(&self, id: &str) {
        self.stops.lock().unwrap().remove(id);
    }
}

pub(crate) fn vectors() -> VectorStore {
    VectorStore::new(data_dir().join("indexes"))
}

/// Runs model work on a thread of its own, so the window stays responsive meanwhile.
async fn in_background<T: Send + 'static>(work: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work).await.map_err(|error| format!("The work stopped unexpectedly: {error}"))?
}

/// Passes events to the window until the request is stopped or the window is gone.
fn sender(on_event: Channel<Value>, stop: Arc<AtomicBool>) -> impl FnMut(Value) -> Flow {
    move |event| {
        if stop.load(Ordering::SeqCst) || on_event.send(event).is_err() {
            Flow::Stop
        } else {
            Flow::Continue
        }
    }
}

#[tauri::command]
pub async fn ai_status(models: State<'_, Models>) -> Result<ai::Status, String> {
    let llm = models.llm.clone();
    in_background(move || with_library(|conn| ai::status(conn, llm.as_ref()))).await
}

#[tauri::command]
pub async fn ask(models: State<'_, Models>, id: String, book_id: i64, question: String, section_id: Option<i64>, on_event: Channel<Value>) -> Result<(), String> {
    let (llm, stop) = (models.llm.clone(), models.begin(&id));
    let answered = in_background(move || {
        let mut emit = sender(on_event, stop);
        with_library(|conn| answer::ask(conn, llm.as_ref(), &vectors(), book_id, &question, section_id, &mut emit))
    }).await;
    models.end(&id);
    answered
}

#[tauri::command]
pub async fn model_pull(models: State<'_, Models>, id: String, name: String, on_event: Channel<Value>) -> Result<(), String> {
    let (llm, stop) = (models.llm.clone(), models.begin(&id));
    let pulled = in_background(move || {
        let mut emit = sender(on_event, stop);
        ai::pull_model(llm.as_ref(), &name, &mut emit).map_err(|error| error.to_string())
    }).await;
    models.end(&id);
    pulled
}

/// Stops a request started with the same id. Stopping one that has finished does nothing.
#[tauri::command]
pub fn stop_request(models: State<'_, Models>, id: String) {
    if let Some(stop) = models.stops.lock().unwrap().get(&id) {
        stop.store(true, Ordering::SeqCst);
    }
}

#[tauri::command]
pub async fn model_delete(models: State<'_, Models>, name: String) -> Result<(), String> {
    let llm = models.llm.clone();
    in_background(move || ai::delete_model(llm.as_ref(), &name).map_err(|error| error.to_string())).await
}

#[tauri::command(async)]
pub fn book_concepts(id: i64) -> Result<Vec<concepts::ConceptView>, String> {
    with_library(|conn| concepts::book_concepts(conn, id, 200))
}

#[derive(serde::Serialize)]
pub struct Enriched {
    added: usize,
}

#[tauri::command]
pub async fn concepts_enrich(models: State<'_, Models>, chapter_id: i64) -> Result<Enriched, String> {
    let llm = models.llm.clone();
    in_background(move || {
        with_library(|conn| {
            let (model, options) = ai::chat_setup(conn, llm.as_ref())?;
            Ok(Enriched { added: concepts::enrich_chapter(conn, llm.as_ref(), chapter_id, &model, &options)? })
        })
    }).await
}

#[tauri::command]
pub async fn section_draw(models: State<'_, Models>, section_id: i64) -> Result<Value, String> {
    let llm = models.llm.clone();
    in_background(move || {
        with_library(|conn| {
            let (model, options) = ai::chat_setup(conn, llm.as_ref())?;
            diagram::draw_section(conn, llm.as_ref(), section_id, &model, &options)
        })
    }).await
}
