//! Running *Weather* (F13).
//!
//! `currency.rs`'s shape: the decisions are in `lumen_core::weather::session`,
//! where they are tested; this owns the requests, the settling timer and
//! the files.
//!
//! # Nothing waits
//!
//! The places, *My Places* and the kept forecasts are read on the
//! background executor; a request runs on a thread of its own, because the
//! client blocks for as long as the service takes. What arrives is merged
//! into `weather.json` -- read again first, since the root list may have
//! written it since -- and handed to the launcher for its answers.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::{mpsc, oneshot};
use gpui::App;
use lumen_core::places::{Catalogue, Favourites};
use lumen_core::view::Event;
use lumen_core::weather::session::{Effect, Session};
use lumen_core::weather::{self, Cache, Failure, Forecast, WeatherConfig};
use lumen_platform::{DateStyle, Platform};
use lumen_ui::{Launcher, ViewSink};

/// How long typing must pause before the cities it lists are asked for.
const SETTLE: Duration = Duration::from_millis(400);

/// Where the files are.
#[derive(Clone)]
pub struct Files {
    pub favourites: PathBuf,
    pub cache: PathBuf,
}

/// Everything the session is told, on one channel so it has one order.
enum Input {
    View(Event),
    /// The view is gone: its sink was dropped.
    Closed,
    Settled(u64),
    Fetched(Vec<(u32, Result<Forecast, Failure>)>),
}

fn now() -> i64 {
    lumen_core::clock::now().as_second()
}

/// Open the Weather view, on `city`'s report when one is named.
pub fn run(
    platform: Arc<dyn Platform>,
    config: WeatherConfig,
    files: Files,
    city: Option<String>,
    window: gpui::WindowHandle<Launcher>,
    cx: &mut App,
) {
    let (view_tx, view_rx) = mpsc::unbounded::<Event>();
    let (tx, rx) = mpsc::unbounded::<Input>();
    let views = view_rx
        .map(Input::View)
        .chain(futures::stream::once(futures::future::ready(Input::Closed)));
    let mut inputs = futures::stream::select(views, rx);

    let sink = ViewSink::new(move |event| {
        let _ = view_tx.unbounded_send(event);
    });
    window
        .update(cx, |launcher, _window, cx| launcher.begin_view(sink, cx))
        .ok();

    cx.spawn(async move |cx| {
        let (places, here, kept, cache) = {
            let files = files.clone();
            cx.background_executor()
                .spawn(async move {
                    (
                        lumen_core::places::catalogue(),
                        lumen_core::clock::here().iana_name().map(str::to_string),
                        Favourites::load(&files.favourites),
                        Cache::load(&files.cache),
                    )
                })
                .await
        };

        let format_platform = platform.clone();
        let time = Box::new(move |when: &lumen_platform::LocalTime| {
            format_platform
                .format_date(when, DateStyle::Time, None)
                .unwrap_or_else(|_| format!("{:02}:{:02}", when.hour, when.minute))
        });
        let mut session = Session::new(config, places, kept, cache, here.as_deref(), time);
        let mut effects = session.start(now());
        if let Some(index) = city.as_deref().and_then(|key| places.city_by_key(key)) {
            effects.extend(session.open(index, now()));
        }

        loop {
            for effect in effects.drain(..) {
                match effect {
                    Effect::Render(view) => {
                        window
                            .update(cx, |launcher, _window, cx| launcher.show_view(view, cx))
                            .ok();
                    }
                    Effect::ClearQuery => {
                        window
                            .update(cx, |launcher, _window, cx| {
                                launcher.set_query(String::new(), cx)
                            })
                            .ok();
                    }
                    Effect::Settle(generation) => {
                        let tx = tx.clone();
                        let timer = cx.background_executor().timer(SETTLE);
                        cx.background_executor()
                            .spawn(async move {
                                timer.await;
                                let _ = tx.unbounded_send(Input::Settled(generation));
                            })
                            .detach();
                    }
                    Effect::Fetch(cities) => {
                        let answer = fetch(places, cities.clone(), files.cache.clone());
                        let tx = tx.clone();
                        cx.spawn(async move |cx| {
                            let (answers, cache) = answered(answer, &cities).await;
                            if let Some(cache) = cache {
                                window
                                    .update(cx, |launcher, _window, cx| {
                                        launcher.set_weather(cache, cx)
                                    })
                                    .ok();
                            }
                            let _ = tx.unbounded_send(Input::Fetched(answers));
                        })
                        .detach();
                    }
                    Effect::Copy(text) => {
                        if let Err(error) = platform.set_clipboard_text(&text) {
                            log::error!("could not copy the weather: {error:#}");
                            continue;
                        }
                        window
                            .update(cx, |launcher, _window, cx| launcher.close(cx))
                            .ok();
                    }
                    Effect::Save(kept) => {
                        let path = files.favourites.clone();
                        cx.background_executor()
                            .spawn(async move {
                                if let Err(error) = kept.save(&path) {
                                    log::warn!("could not keep My Places: {error:#}");
                                }
                            })
                            .detach();
                    }
                }
            }

            let Some(input) = inputs.next().await else {
                break;
            };
            effects = match input {
                Input::View(event) => session.event(event, now()),
                Input::Closed => break,
                Input::Settled(generation) => session.settled(generation, now()),
                Input::Fetched(answers) => session.fetched(answers),
            };
        }
    })
    .detach();
}

/// Fetch one city's forecast for `weather in ...` in the root list (F13).
/// The forecasts go back to the launcher, or why not.
pub fn refresh(city: u32, cache: PathBuf, window: gpui::WindowHandle<Launcher>, cx: &mut App) {
    cx.spawn(async move |cx| {
        let places = cx
            .background_executor()
            .spawn(async { lumen_core::places::catalogue() })
            .await;
        let Some(key) = places.city(city).map(|found| places.key(found)) else {
            return;
        };
        let (answers, kept) = answered(fetch(places, vec![city], cache), &[city]).await;
        window
            .update(cx, |launcher, _window, cx| {
                if let Some(kept) = kept {
                    launcher.set_weather(kept, cx);
                }
                if let Some((_, Err(failure))) = answers.into_iter().next() {
                    launcher.weather_failed(key, failure, cx);
                }
            })
            .ok();
    })
    .detach();
}

type Answers = (Vec<(u32, Result<Forecast, Failure>)>, Option<Cache>);

/// What [`fetch`] answered; a thread that never started fails every city,
/// so none is left loading.
async fn answered(answer: oneshot::Receiver<Answers>, cities: &[u32]) -> Answers {
    answer.await.unwrap_or_else(|_| {
        let failure = Failure::Other("Could not start a request".into());
        (
            cities
                .iter()
                .map(|index| (*index, Err(failure.clone())))
                .collect(),
            None,
        )
    })
}

/// Fetch `cities` in one request on a thread of its own; merge what came
/// into the cache file. The answer is one result per city, and the cache
/// as written when anything arrived.
fn fetch(
    places: &'static Catalogue,
    cities: Vec<u32>,
    path: PathBuf,
) -> oneshot::Receiver<Answers> {
    let (tx, rx) = oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name("lumen-weather".into())
        .spawn(move || {
            let found: Vec<_> = cities
                .iter()
                .filter_map(|index| places.city(*index).map(|city| (*index, city)))
                .collect();
            let coordinates: Vec<(f32, f32)> = found
                .iter()
                .map(|(_, city)| (city.latitude, city.longitude))
                .collect();
            let answers: Vec<(u32, Result<Forecast, Failure>)> =
                match weather::source::fetch(&coordinates, now()) {
                    Ok(forecasts) => {
                        log::info!("fetched {} forecasts", forecasts.len());
                        found
                            .iter()
                            .map(|(index, _)| *index)
                            .zip(forecasts.into_iter().map(Ok))
                            .collect()
                    }
                    Err(error) => {
                        log::warn!("could not fetch the weather: {error:#}");
                        let failure = weather::failure(&error);
                        cities
                            .iter()
                            .map(|index| (*index, Err(failure.clone())))
                            .collect()
                    }
                };

            let mut kept = None;
            if answers.iter().any(|(_, answer)| answer.is_ok()) {
                let mut cache = Cache::load(&path);
                for (index, answer) in &answers {
                    if let (Ok(forecast), Some(city)) = (answer, places.city(*index)) {
                        cache.insert(places.key(city), forecast.clone());
                    }
                }
                if let Err(error) = cache.save(&path) {
                    log::warn!("could not keep the forecasts: {error:#}");
                }
                kept = Some(cache);
            }
            let _ = tx.send((answers, kept));
        });
    if let Err(error) = spawned {
        log::error!("could not start a request for the weather: {error}");
    }
    rx
}
