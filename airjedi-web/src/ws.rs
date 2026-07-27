use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::config::FeedConfig;
use crate::dto::{ClientMessage, ServerMessage};
use crate::state::SharedState;

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<SharedState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: SharedState) {
    let (mut sender, mut receiver) = socket.split();

    {
        let feeds = state.feeds.lock().await;
        let aircraft = feeds.get_all_aircraft();
        let msg = ServerMessage::Snapshot { aircraft };
        if let Ok(json) = serde_json::to_string(&msg) {
            if sender.send(Message::Text(json.into())).await.is_err() {
                return;
            }
        }
    }

    let mut broadcast_rx = state.broadcast_tx.subscribe();

    let mut send_task = tokio::spawn(async move {
        loop {
            match broadcast_rx.recv().await {
                Ok(msg) => {
                    if let Ok(json) = serde_json::to_string(&msg) {
                        if sender.send(Message::Text(json.into())).await.is_err() {
                            break;
                        }
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("WebSocket client lagged by {n} messages");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    let state_clone = state.clone();
    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            if let Message::Text(text) = msg {
                if let Ok(cmd) = serde_json::from_str::<ClientMessage>(&text) {
                    handle_client_message(cmd, &state_clone).await;
                }
            }
        }
    });

    tokio::select! {
        _ = &mut send_task => { recv_task.abort(); },
        _ = &mut recv_task => { send_task.abort(); },
    }
}

async fn handle_client_message(msg: ClientMessage, state: &SharedState) {
    let mut feeds = state.feeds.lock().await;
    match msg {
        ClientMessage::AddFeed { address, protocol } => {
            feeds.add_feed(FeedConfig {
                id: Uuid::new_v4(),
                address,
                protocol,
            });
        }
        ClientMessage::RemoveFeed { id } => {
            feeds.remove_feed(id);
        }
    }
}
