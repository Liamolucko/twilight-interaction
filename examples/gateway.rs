use std::env;
use std::sync::Arc;

use futures::FutureExt;
use futures::StreamExt;
use rand::thread_rng;
use rand::Rng;
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
