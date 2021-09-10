use std::env;
use std::sync::Arc;

use commands::build_handler;
use futures::StreamExt;
use twilight_gateway::Cluster;
use twilight_gateway::Event;
use twilight_gateway::EventTypeFlags;
use twilight_gateway::Intents;
use twilight_http::Client;

#[path = "common/commands.rs"]
mod commands;

#[tokio::main]
async fn main() {
    env_logger::init();

    let token = env::var("TOKEN").expect("Missing discord bot token");

    let http = Client::new(token.clone());
    http.set_application_id(
        env::var("APP_ID")
            .expect("Missing application ID")
            .parse::<u64>()
            .unwrap()
            .into(),
    );

    let guild_id = env::var("GUILD_ID")
        .expect("Missing guild ID")
        .parse::<u64>()
        .unwrap()
        .into();

    let handler = build_handler(guild_id, http.clone()).await;

    let handler = Arc::new(handler);

    let (cluster, mut events) = Cluster::builder(token, Intents::empty())
        .event_types(EventTypeFlags::INTERACTION_CREATE)
        .http_client(http.clone())
        .build()
        .await
        .expect("failed to start cluster");

    tokio::spawn(async move {
        cluster.up().await;
    });

    while let Some((_, event)) = events.next().await {
        match event {
            Event::InteractionCreate(event) => {
                let handler = Arc::clone(&handler);
                tokio::spawn(async move {
                    if let Err(err) = handler.handle_event(*event).await {
                        log::error!("{}", err);
                    }
                });
            }
            // Ignore any other events
            _ => {}
        }
    }
}
