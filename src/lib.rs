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
pub enum CommandError {
    #[error("Unknown command '/{0}'.")]
    Unknown(String),
    // We don't need to be too specific about options being wrong, since something had to have gone terribly wrong for this to happen.
    // We check the IDs of the commands, so we know they're exactly the commands they're supposed to be,
    // so they could only really have invalid options if something went wrong on Discord's end.
    #[error("Invalid option '{0}'.")]
    InvalidOption(&'static str),
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
    #[cfg(feature = "webhook")]
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
}

pub struct CommandDecl {
    pub handler: Box<
        dyn Fn(
                Vec<CommandDataOption>,
                Option<CommandInteractionDataResolved>,
            ) -> Result<CommandResponse, CommandError>
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
            ) -> Result<CommandResponse, CommandError>
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

    pub fn handle(&self, command: CommandData) -> Result<CommandResponse, CommandError> {
        for handler in &self.handlers {
            if handler.id == command.id {
                return (handler.handler)(command.options, command.resolved);
            }
        }

        // It didn't match any known commands, so fail.
        Err(CommandError::Unknown(command.name))
    }

    /// Handle an INTERACTION_CREATE event from the Discord Gateway, automatically sending the response over HTTP.
    ///
    /// Requires the `gateway` feature to be enabled.
    #[cfg(feature = "gateway")]
    pub async fn handle_gateway(&self, command: ApplicationCommand) -> Result<(), Error> {
        use twilight_model::application::callback::InteractionResponse;

        use crate::_macro_internal::InteractionResult;

        match self.handle(command.data) {
            Ok(response) => match response {
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

                    builder.exec().await?;
                }
            },
            Err(e) => {
                // This shouldn't happen, but provide a reasonable response.
                self.http
                    .interaction_callback(
                        command.id,
                        &command.token,
                        &InteractionResponse::ChannelMessageWithSource(
                            format!("Error: {}", e).into_callback_data(),
                        ),
                    )
                    .exec()
                    .await?;
            }
        }
        Ok(())
    }

    #[cfg(feature = "webhook")]
    pub async fn handle_request(
        &self,
        request: http::Request<&[u8]>,
        pub_key: &ed25519_dalek::PublicKey,
    ) -> Result<
        (
            http::Response<Vec<u8>>,
            Option<Pin<Box<dyn Future<Output = Result<(), Error>> + Send>>>,
        ),
        Error,
    > {
        use http::header::CONTENT_TYPE;
        use http::Response;
        use http::StatusCode;
        use twilight_model::application::callback::InteractionResponse;
        use twilight_model::application::interaction::Interaction;

        use crate::_macro_internal::InteractionResult;

        let interaction = match process(request, pub_key) {
            Ok(interaction) => interaction,
            Err(status) => {
                return Ok((
                    // This can never fail, so it's fine to `unwrap` it -
                    // `status` only fails if it fails to convert to a `StatusCode`, but it's already a `StatusCode`,
                    // and `body` never fails.
                    Response::builder().status(status).body(vec![]).unwrap(),
                    None,
                ));
            }
        };

        match interaction {
            // Return a Pong if a Ping is received.
            Interaction::Ping(_) => {
                let response = InteractionResponse::Pong;

                let json = serde_json::to_vec(&response)?;

                Ok((
                    Response::builder()
                        .status(StatusCode::OK)
                        .header(CONTENT_TYPE, "application/json")
                        .body(json.into())
                        // If this is going to fail, it will always fail.
                        .unwrap(),
                    None,
                ))
            }
            // Respond to a slash command.
            Interaction::ApplicationCommand(command) => {
                // Run the handler to gain a response.
                let (response, fut) = match self.handle(command.data) {
                    Ok(response) => match response {
                        CommandResponse::Sync(res) => {
                            (InteractionResponse::ChannelMessageWithSource(res), None)
                        }
                        CommandResponse::Async(fut) => (
                            InteractionResponse::ChannelMessageWithSource(fut.await),
                            None,
                        ),
                        CommandResponse::Deferred(fut) => {
                            let token = command.token;

                            let http = self.http.clone();

                            let fut = Box::pin(async move {
                                let res = fut.await;

                                let mut builder = http
                                    .update_interaction_original(&token)?
                                    .content(res.content.as_deref())?
                                    .embeds(Some(&res.embeds))?;

                                if let Some(allowed_mentions) = res.allowed_mentions {
                                    builder = builder.allowed_mentions(allowed_mentions);
                                }

                                builder.exec().await?;

                                Ok(())
                            }) as _;

                            let response = InteractionResponse::DeferredChannelMessageWithSource(
                                CallbackData {
                                    allowed_mentions: None,
                                    content: None,
                                    embeds: vec![],
                                    flags: None,
                                    tts: None,
                                    components: None,
                                },
                            );

                            (response, Some(fut))
                        }
                    },
                    Err(e) => (
                        InteractionResponse::ChannelMessageWithSource(
                            format!("Error: {}", e).into_callback_data(),
                        ),
                        None,
                    ),
                };

                // Serialize the response and return it back to discord.
                let json = serde_json::to_vec(&response)?;

                let response = Response::builder()
                    .header(CONTENT_TYPE, "application/json")
                    .status(StatusCode::OK)
                    .body(json.into())
                    .unwrap();

                Ok((response, fut))
            }
            // Unhandled interaction types.
            _ => Ok((
                Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(vec![])
                    .unwrap(),
                None,
            )),
        }
    }
}

/// Get the interaction sent in a request, or return an appropriate error code if it's invalid.
#[cfg(feature = "webhook")]
fn process(
    request: http::Request<&[u8]>,
    pub_key: &ed25519_dalek::PublicKey,
) -> Result<twilight_model::application::interaction::Interaction, http::StatusCode> {
    use ed25519_dalek::Signature;
    use ed25519_dalek::Verifier;
    use hex::FromHex;
    use http::Method;
    use http::StatusCode;
    use twilight_model::application::interaction::Interaction;

    // Check that the method used is a POST, all other methods are not allowed.
    if request.method() != Method::POST {
        return Err(StatusCode::METHOD_NOT_ALLOWED);
    }

    // Extract the timestamp header for use later to check the signature.
    let timestamp = request
        .headers()
        .get("x-signature-timestamp")
        .ok_or(StatusCode::BAD_REQUEST)?;

    // Extact the signature to check against.
    let signature = request
        .headers()
        .get("x-signature-ed25519")
        .ok_or(StatusCode::BAD_REQUEST)?;
    let signature =
        Signature::new(FromHex::from_hex(signature).map_err(|_| StatusCode::BAD_REQUEST)?);

    let body = *request.body();

    // Check if the signature matches and else return a error response.
    pub_key
        .verify([timestamp.as_bytes(), body].concat().as_ref(), &signature)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    // Deserialize the body into a interaction.
    serde_json::from_slice::<Interaction>(body).map_err(|_| StatusCode::BAD_REQUEST)
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
