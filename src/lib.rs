use std::collections::HashMap;
use std::convert::TryFrom;
use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;

use thiserror::Error;
use twilight_http::request::application::InteractionError;
use twilight_http::request::application::UpdateOriginalResponseError;
use twilight_http::response::DeserializeBodyError;
use twilight_http::Client;
use twilight_model::application::callback::CallbackData;
use twilight_model::application::command::Command;
use twilight_model::application::command::CommandOption;
use twilight_model::application::interaction::application_command::CommandData;
use twilight_model::application::interaction::application_command::CommandDataOption;
use twilight_model::application::interaction::application_command::CommandInteractionDataResolved;
use twilight_model::application::interaction::ApplicationCommand;
use twilight_model::guild::Role;
use twilight_model::id::CommandId;
use twilight_model::id::GuildId;
use twilight_model::user::User;

#[doc(hidden)]
pub mod _macro_internal;

pub use twilight_slash_command_macros::*;

/// Anything which can be mentioned; either a user or a role.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum Mentionable {
    User(User),
    Role(Role),
}

/// A trait to be implemented for C-like enums of choices for users to enter as arguments to your interaction.
///
/// You should usually just implement this by deriving it.
///
/// # Examples
/// ```
/// use twilight_slash_command::Choices;
/// use num_enum::{TryFromPrimitive, IntoPrimitive};
///
/// #[repr(i64)]
/// #[derive(IntoPrimitive, TryFromPrimitive, Choices)]
/// enum Foo {
///     Bar,
///     Baz,
///     #[name = "not an ident!"]
///     Qux,
/// }
///
/// assert_eq!(
///     Foo::CHOICES,
///     &[("Bar", 0), ("Baz", 1), ("not an ident!", 2)]
/// );
pub trait Choices: Into<i64> + TryFrom<i64> {
    const CHOICES: &'static [(&'static str, i64)];
}

pub enum CommandResponse {
    /// This slash command handler is synchronous; here's the response.
    Sync(CallbackData),
    /// This slash command handler is asynchronous; await this future to get the response.
    Async(Pin<Box<dyn Future<Output = CallbackData> + Send>>),
    /// This slash command is deferred; return a deferred response now, and then update the message when this future completes.
    Deferred(Pin<Box<dyn Future<Output = CallbackData> + Send>>),
}

#[derive(Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Interaction(#[from] InteractionError),
    #[error(transparent)]
    Http(#[from] twilight_http::Error),
    #[error(transparent)]
    Deserialize(#[from] DeserializeBodyError),
    #[error(transparent)]
    UpdateResponse(#[from] UpdateOriginalResponseError),
}

pub struct CommandDecl {
    pub handler: Box<
        dyn Fn(
                Vec<CommandDataOption>,
                Option<CommandInteractionDataResolved>,
            ) -> Option<CommandResponse>
            + Send
            + Sync,
    >,
    pub name: &'static str,
    pub description: &'static str,
    pub options: Vec<CommandOption>,
}

impl CommandDecl {
    fn description(&self) -> Command {
        Command {
            // These are only included on responses
            application_id: None,
            guild_id: None,
            id: None,

            // TODO: support setting this
            default_permission: None,

            name: self.name.to_string(),
            description: self.description.to_string(),
            options: self.options.clone(),
        }
    }
}

/// The information needed to actually handle a command.
struct CommandHandler {
    handler: Box<
        dyn Fn(
                Vec<CommandDataOption>,
                Option<CommandInteractionDataResolved>,
            ) -> Option<CommandResponse>
            + Send
            + Sync,
    >,
    id: CommandId,
}

pub struct Handler {
    http: Client,
    handlers: Vec<CommandHandler>,
}

impl Handler {
    pub fn builder(http: Client) -> HandlerBuilder {
        HandlerBuilder {
            global_commands: Vec::new(),
            guild_commands: HashMap::new(),
            http,
        }
    }

    pub fn handle(&self, command: CommandData) -> Option<CommandResponse> {
        for handler in &self.handlers {
            if handler.id == command.id {
                return (handler.handler)(command.options, command.resolved);
            }
        }

        // It didn't match any known commands, so fail.
        None
    }

    /// Handle an INTERACTION_CREATE event from the Discord Gateway, automatically sending the response over HTTP.
    ///
    /// Requires the `gateway` feature to be enabled.
    #[cfg(feature = "gateway")]
    pub async fn handle_gateway(&self, command: ApplicationCommand) -> Result<(), Error> {
        use twilight_model::application::callback::InteractionResponse;

        use crate::_macro_internal::InteractionResult;

        match self.handle(command.data) {
            Some(response) => match response {
                CommandResponse::Sync(res) => {
                    self.http
                        .interaction_callback(
                            command.id,
                            &command.token,
                            &InteractionResponse::ChannelMessageWithSource(res),
                        )
                        .exec()
                        .await?;
                }
                CommandResponse::Async(fut) => {
                    let res = fut.await;

                    self.http
                        .interaction_callback(
                            command.id,
                            &command.token,
                            &InteractionResponse::ChannelMessageWithSource(res),
                        )
                        .exec()
                        .await?;
                }
                CommandResponse::Deferred(fut) => {
                    self.http
                        .interaction_callback(
                            command.id,
                            &command.token,
                            // I'm pretty sure the fact that this takes `CallbackData` in the first place is a mistake; it doesn't do anything.
                            &InteractionResponse::DeferredChannelMessageWithSource(CallbackData {
                                allowed_mentions: None,
                                content: None,
                                embeds: vec![],
                                flags: None,
                                tts: None,
                                components: None,
                            }),
                        )
                        .exec()
                        .await?;

                    let res = fut.await;

                    let mut builder = self
                        .http
                        .update_interaction_original(&command.token)?
                        .content(res.content.as_deref())?
                        .embeds(Some(&res.embeds))?;

                    if let Some(allowed_mentions) = res.allowed_mentions {
                        builder = builder.allowed_mentions(allowed_mentions);
                    }

                    builder.exec().await.unwrap();
                }
            },
            None => {
                // This should never happen, but we can't just 400 like if this was handling webhooks so provide a reasonable response.
                self.http
                    .interaction_callback(
                        command.id,
                        &command.token,
                        &InteractionResponse::ChannelMessageWithSource(
                            "Unexpected interaction received"
                                .to_string()
                                .into_callback_data(),
                        ),
                    )
                    .exec()
                    .await?;
            }
        }
        Ok(())
    }
}

pub struct HandlerBuilder {
    global_commands: Vec<CommandDecl>,
    guild_commands: HashMap<GuildId, Vec<CommandDecl>>,
    http: Client,
}

impl HandlerBuilder {
    pub fn global_command(mut self, command: CommandDecl) -> Self {
        self.global_commands.push(command);
        self
    }

    pub fn guild_command(mut self, guild_id: GuildId, command: CommandDecl) -> Self {
        let guild_commands = self.guild_commands.entry(guild_id).or_insert(vec![]);
        guild_commands.push(command);
        self
    }

    /// Registers the slash commands with Discord and returns the `Handler` to handle them.
    pub async fn build(self) -> Result<Handler, Error> {
        let mut handlers = Vec::new();

        // TODO: do this in parallel with the guild commands.
        let response = self
            .http
            .set_global_commands(
                &self
                    .global_commands
                    .iter()
                    .map(CommandDecl::description)
                    .collect::<Vec<_>>(),
            )?
            .exec()
            .await?
            .models()
            .await?;

        for (command, description) in self.global_commands.into_iter().zip(response.into_iter()) {
            handlers.push(CommandHandler {
                id: description.id.unwrap(),
                handler: command.handler,
            })
        }

        for (guild_id, commands) in self.guild_commands.into_iter() {
            let response = self
                .http
                .set_guild_commands(
                    guild_id,
                    &commands
                        .iter()
                        .map(CommandDecl::description)
                        .collect::<Vec<_>>(),
                )?
                .exec()
                .await?
                .models()
                .await?;

            for (command, description) in commands.into_iter().zip(response.into_iter()) {
                handlers.push(CommandHandler {
                    id: description.id.unwrap(),
                    handler: command.handler,
                })
            }
        }

        Ok(Handler {
            http: self.http,
            handlers,
        })
    }
}
