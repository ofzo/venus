use crossterm::event::{self as cevent, Event as CEvent, KeyEvent, MouseEvent};
use tokio::sync::mpsc;
use venus_core::stream::StreamEvent;

/// Events processed by the TUI event loop.
pub enum AppEvent {
    /// Keyboard input.
    Key(KeyEvent),
    /// Mouse input (for scroll).
    Mouse(MouseEvent),
    /// Terminal resize.
    #[allow(dead_code)]
    Resize(u16, u16),
    /// Tick for animation (spinner, 100ms interval).
    Tick,
    /// Streaming event from the engine.
    Stream(StreamEvent),
    /// Cron-scheduled prompt to execute.
    CronPrompt(String),
}

/// Spawn a crossterm event polling thread and a tick timer task.
/// Returns a receiver that yields `AppEvent`s.
pub fn spawn_event_poller() -> mpsc::UnboundedReceiver<AppEvent> {
    let (tx, rx) = mpsc::unbounded_channel();

    // Crossterm event polling thread (blocking)
    let key_tx = tx.clone();
    std::thread::spawn(move || {
        loop {
            if cevent::poll(std::time::Duration::from_millis(100)).unwrap_or(false) {
                match cevent::read() {
                    Ok(CEvent::Key(key)) if key_tx.send(AppEvent::Key(key)).is_err() => {
                        break;
                    }
                    Ok(CEvent::Mouse(mouse)) if key_tx.send(AppEvent::Mouse(mouse)).is_err() => {
                        break;
                    }
                    Ok(CEvent::Resize(w, h)) if key_tx.send(AppEvent::Resize(w, h)).is_err() => {
                        break;
                    }
                    _ => {}
                }
            }
        }
    });

    // Tick timer (100ms for spinner animation)
    let tick_tx = tx.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(100));
        loop {
            interval.tick().await;
            if tick_tx.send(AppEvent::Tick).is_err() {
                break;
            }
        }
    });

    rx
}

/// Forward StreamEvents from an engine receiver into the AppEvent channel.
pub fn spawn_stream_forwarder(
    mut stream_rx: mpsc::UnboundedReceiver<StreamEvent>,
    event_tx: mpsc::UnboundedSender<AppEvent>,
) {
    tokio::spawn(async move {
        while let Some(event) = stream_rx.recv().await {
            if event_tx.send(AppEvent::Stream(event)).is_err() {
                break;
            }
        }
    });
}
