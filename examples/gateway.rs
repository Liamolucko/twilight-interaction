use std::env;
use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;
use futures::StreamExt;
use rand::thread_rng;
use rand::Rng;
use serde::Deserialize;
use twilight_gateway::Cluster;
use twilight_gateway::Event;
use twilight_gateway::EventTypeFlags;
use twilight_gateway::Intents;
use twilight_http::Client;
use twilight_mention::Mention;
use twilight_model::application::callback::InteractionResponse;
use twilight_model::application::interaction::application_command::InteractionChannel;
use twilight_model::application::interaction::Interaction;
use twilight_model::guild::Role;
use twilight_model::user::User;
use twilight_slash_command::slash_command;
use twilight_slash_command::Handler;
use twilight_slash_command::Mentionable;

#[slash_command(description("Frobs some bits", bits = "The bits to frob"))]
fn frob(bits: i64) -> String {
    bits.reverse_bits().to_string()
}

#[slash_command(description("Generate a random number from 1 to 10"))]
fn random() -> String {
    thread_rng().gen_range(1..=10).to_string()
}

#[slash_command(description(
    "Takes all the args",
    string = "A string",
    int = "An int",
    bool = "A bool",
    user = "A user",
    channel = "A channel",
    role = "A role",
    mentionable = "Something mentionable"
))]
fn all_the_args(
    string: String,
    int: i64,
    bool: bool,
    user: User,
    channel: InteractionChannel,
    role: Role,
    mentionable: Mentionable,
) -> String {
    format!(
        "string: {:?}
int: {},
bool: {},
user: {},
channel: {},
role: {},
mentionable: {}",
        string,
        int,
        bool,
        user.mention(),
        channel.id.mention(),
        role.mention(),
        // Sadly, it isn't possible to integrate `Mentionable` properly with `twilight-mention`.
        // I guess I could add a `mention` method which just returns a string though.
        match mentionable {
            Mentionable::User(user) => user.mention().to_string(),
            Mentionable::Role(role) => role.mention().to_string(),
        }
    )
}

#[slash_command(defer, description("Prints 'Hello!' after 1 second."))]
async fn greet() -> String {
    tokio::time::sleep(Duration::from_secs(1)).await;
    "Hello!".to_string()
}

#[slash_command(description("Gets the current Rust version"))]
async fn rust_version() -> String {
    // The subset of the manifest we care about.
    #[derive(Deserialize)]
    struct Manifest {
        pkg: Packages,
    }
    #[derive(Deserialize)]
    struct Packages {
        rust: Package,
    }
    #[derive(Deserialize)]
    struct Package {
        version: String,
    }

    let text = async {
        reqwest::get("https://static.rust-lang.org/dist/channel-rust-stable.toml")
            .await?
            .text()
            .await
    };
    match text.await {
        Ok(text) => match toml::from_str::<Manifest>(&text) {
            Ok(manifest) => manifest.pkg.rust.version,
            Err(err) => format!("Error parsing TOML: {}", err),
        },
        Err(err) => format!("Network error: {}", err),
    }
}

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
