use std::env;
use std::sync::Arc;

use futures::FutureExt;
use futures::StreamExt;
use twilight_gateway::Cluster;
use twilight_gateway::Event;
use twilight_gateway::EventTypeFlags;
use twilight_gateway::Intents;
use twilight_http::Client;
use twilight_model::application::callback::InteractionResponse;
use twilight_model::application::interaction::Interaction;
use twilight_slash_command::Handler;

#[path = "common/commands.rs"]
mod commands;

use commands::{all_the_args, frob, greet, random, rust_version};

#[tokio::main]
async fn main() {
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

    let handler = Handler::builder(http.clone())
        .guild_command(guild_id, frob::describe())
        .guild_command(guild_id, random::describe())
        .guild_command(guild_id, all_the_args::describe())
        .guild_command(guild_id, greet::describe())
        .guild_command(guild_id, rust_version::describe())
        .build()
        .await
        .unwrap();

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
            Event::InteractionCreate(event) => match event.0 {
                Interaction::Ping(ping) => {
                    // I'm pretty sure Discord never sends pings over the gateway, but we may as well handle it properly.
                    tokio::spawn(
                        http.interaction_callback(ping.id, &ping.token, &InteractionResponse::Pong)
                            .exec()
                            .map(Result::unwrap),
                    );
                }
                Interaction::ApplicationCommand(command) => {
                    let handler = Arc::clone(&handler);
                    tokio::spawn(async move {
                        handler.handle_gateway(*command).await.unwrap();
                    });
                }
                _ => {}
            },
            // Ignore any other events
            _ => {}
        }
    }
}
