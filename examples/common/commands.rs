use std::time::Duration;

use rand::thread_rng;
use rand::Rng;
use serde::Deserialize;
use twilight_mention::Mention;
use twilight_model::application::interaction::application_command::InteractionChannel;
use twilight_model::guild::Role;
use twilight_model::user::User;
use twilight_slash_command::slash_command;
use twilight_slash_command::Choices;
use twilight_slash_command::Mentionable;

#[slash_command(description("Frobs some bits", bits = "The bits to frob"))]
pub fn frob(bits: i64) -> String {
    bits.reverse_bits().to_string()
}

#[slash_command(description("Generate a random number from 1 to 10"))]
pub fn random() -> String {
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
pub fn all_the_args(
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
pub async fn greet() -> String {
    tokio::time::sleep(Duration::from_secs(1)).await;
    "Hello!".to_string()
}

#[slash_command(description("Gets the current Rust version"))]
pub async fn rust_version() -> String {
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

#[derive(Choices)]
pub enum Type {
    #[name = "bool"]
    Bool,
    #[name = "char"]
    Char,
    Duration,
    #[name = "i32"]
    I32,
    Option,
    String,
    Vec,
}

#[slash_command(
    description(
        "Gets the default value for a type",
        type_option = "The type to get the default value of"
    ),
    rename(type_option = "type")
)]
pub fn default(type_option: Type) -> String {
    match type_option {
        Type::Bool => format!("`{:?}`", bool::default()),
        Type::Char => format!("`{:?}`", char::default()),
        Type::Duration => format!("`{:?}`", Duration::default()),
        Type::I32 => format!("`{:?}`", i32::default()),
        Type::Option => format!("`{:?}`", Option::<()>::default()),
        Type::String => format!("`{:?}`", String::default()),
        Type::Vec => format!("`{:?}`", Vec::<()>::default()),
    }
}
