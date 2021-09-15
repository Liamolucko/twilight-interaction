use std::collections::HashMap;
use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;

use thiserror::Error;
use twilight_http::request::application::interaction::update_original_response::UpdateOriginalResponseError;
use twilight_http::request::application::InteractionError;
use twilight_http::response::DeserializeBodyError;
use twilight_http::Client;
use twilight_model::application::callback::CallbackData;
use twilight_model::application::callback::InteractionResponse;
use twilight_model::application::command::Command;
use twilight_model::application::command::CommandOption;
use twilight_model::application::command::CommandType;
use twilight_model::application::interaction::application_command::CommandData;
use twilight_model::application::interaction::application_command::CommandDataOption;
use twilight_model::application::interaction::application_command::CommandInteractionDataResolved;
use twilight_model::application::interaction::message_component::MessageComponentInteractionData;
use twilight_model::application::interaction::Interaction;
use twilight_model::channel::Message;
use twilight_model::id::CommandId;
use twilight_model::id::GuildId;
use twilight_model::id::InteractionId;

mod option_types;

pub use twilight_interaction_macros::slash_command;
// Only show the trait in docs, not the derive macro.
pub use option_types::*;
#[doc(hidden)]
pub use twilight_interaction_macros::Choices;
use twilight_model::user::User;

/// An empty `CallbackData`, to use for the pointless field of `InteractionResponse::DeferredChannelMessageWithSource`.
const EMPTY_CALLBACK: CallbackData = CallbackData {
    allowed_mentions: None,
    components: None,
    content: None,
    embeds: vec![],
    flags: None,
    tts: None,
};

pub enum ComponentResponse {
    Message(CallbackData),
    DeferredMessage(DeferredFuture),
    Update(CallbackData),
    DeferredUpdate(DeferredFuture),
}

/// A future for the result of an asynchronous command.
pub type DeferredFuture = Pin<Box<dyn Future<Output = CallbackData> + Send>>;

pub struct Response {
    /// The actual `InteractionResponse` to return to discord.
    response: InteractionResponse,
    /// If the response is deferred, a future to await to get the deferred message.
    future: Option<DeferredFuture>,
    /// The interaction ID extracted from the interaction.
    id: InteractionId,
    /// The interaction token extracted from the interaction.
    token: String,
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

pub enum CommandDecl {
    Slash {
        description: &'static str,
        options: Vec<CommandOption>,
        handler: SlashHandlerFn,
    },
    Message {
        handler: MessageHandlerFn,
    },
    User {
        handler: UserHandlerFn,
    },
}

impl<R: CommandResponse + 'static> From<fn(Message) -> R> for CommandDecl {
    fn from(func: fn(Message) -> R) -> Self {
        CommandDecl::Message {
            handler: Box::new(move |message| func(message).into_interaction_response()),
        }
    }
}

impl<R: CommandResponse + 'static> From<fn(User) -> R> for CommandDecl {
    fn from(func: fn(User) -> R) -> Self {
        CommandDecl::User {
            handler: Box::new(move |user| func(user).into_interaction_response()),
        }
    }
}

impl CommandDecl {
    fn description(&self, name: String) -> Command {
        Command {
            // These are only included on responses
            application_id: None,
            guild_id: None,
            id: None,

            // TODO: support setting this
            default_permission: None,

            name,

            description: if let CommandDecl::Slash { description, .. } = self {
                *description
            } else {
                ""
            }
            .to_string(),

            options: if let CommandDecl::Slash { options, .. } = self {
                options.clone()
            } else {
                vec![]
            },

            kind: match self {
                CommandDecl::Slash { .. } => CommandType::ChatInput,
                CommandDecl::Message { .. } => CommandType::Message,
                CommandDecl::User { .. } => CommandType::User,
            },
        }
    }
}

type SlashHandlerFn = Box<
    dyn Fn(
            Vec<CommandDataOption>,
            Option<CommandInteractionDataResolved>,
        ) -> Result<(InteractionResponse, Option<DeferredFuture>), String>
        + Send
        + Sync,
>;

type MessageHandlerFn =
    Box<dyn Fn(Message) -> (InteractionResponse, Option<DeferredFuture>) + Send + Sync>;

type UserHandlerFn =
    Box<dyn Fn(User) -> (InteractionResponse, Option<DeferredFuture>) + Send + Sync>;

/// The information needed to actually handle a command.
enum CommandHandler {
    Slash(SlashHandlerFn),
    Message(MessageHandlerFn),
    User(UserHandlerFn),
}

impl CommandHandler {
    fn handle(&self, data: CommandData) -> (InteractionResponse, Option<DeferredFuture>) {
        match self {
            Self::Slash(handler) => handler(data.options, data.resolved).unwrap_or_else(|err| {
                (
                    InteractionResponse::ChannelMessageWithSource(
                        format!("Invalid option '{}'", err).into_callback_data(),
                    ),
                    None,
                )
            }),
            // These two are implemented a bit hackily; twilight doesn't expose `target_id` yet,
            // so we have to exploit the fact that the user/message being targeted is the only thing in resolved (hopefully!)
            Self::Message(handler) => data
                .resolved
                .filter(|resolved| resolved.messages.len() == 1)
                .and_then(|mut resolved| resolved.messages.pop())
                .map(&*handler)
                .unwrap_or_else(|| {
                    (
                        InteractionResponse::ChannelMessageWithSource(
                            "Invalid message command recieved".to_string().into_callback_data(),
                        ),
                        None,
                    )
                }),
            Self::User(handler) => data
                .resolved
                .filter(|resolved| resolved.users.len() == 1)
                .and_then(|mut resolved| resolved.users.pop())
                .map(&*handler)
                .unwrap_or_else(|| {
                    (
                        InteractionResponse::ChannelMessageWithSource(
                            "Invalid user command recieved".to_string().into_callback_data(),
                        ),
                        None,
                    )
                }),
        }
    }
}

impl From<CommandDecl> for CommandHandler {
    fn from(decl: CommandDecl) -> Self {
        match decl {
            CommandDecl::Slash { handler, .. } => Self::Slash(handler),
            CommandDecl::Message { handler } => Self::Message(handler),
            CommandDecl::User { handler } => Self::User(handler),
        }
    }
}

pub struct Handler {
    http: Client,
    command_handlers: Vec<(CommandId, CommandHandler)>,
    component_handler: Option<
        Box<dyn Fn(Message, MessageComponentInteractionData) -> ComponentResponse + Send + Sync>,
    >,
}

impl Handler {
    pub fn builder(http: Client) -> HandlerBuilder {
        HandlerBuilder {
            global_commands: Vec::new(),
            guild_commands: HashMap::new(),
            component_handler: None,
            http,
        }
    }

    pub fn handle(&self, interaction: Interaction) -> Response {
        match interaction {
            Interaction::Ping(ping) => Response {
                response: InteractionResponse::Pong,
                future: None,
                id: ping.id,
                token: ping.token,
            },
            Interaction::ApplicationCommand(command) => {
                for (id, handler) in &self.command_handlers {
                    if command.data.id == *id {
                        let (response, future) = handler.handle(command.data);

                        return Response {
                            response,
                            future,
                            id: command.id,
                            token: command.token,
                        };
                    }
                }

                // It didn't match any known commands, so give an error response.
                Response {
                    response: InteractionResponse::ChannelMessageWithSource(
                        format!("Unknown command '/{}'", command.data.name).into_callback_data(),
                    ),
                    future: None,
                    id: command.id,
                    token: command.token,
                }
            }
            Interaction::MessageComponent(interaction) => {
                let (response, future) = if let Some(handler) = &self.component_handler {
                    let response = handler(interaction.message, interaction.data);
                    match response {
                        ComponentResponse::Message(data) => {
                            (InteractionResponse::ChannelMessageWithSource(data), None)
                        }
                        ComponentResponse::DeferredMessage(future) => (
                            InteractionResponse::DeferredChannelMessageWithSource(EMPTY_CALLBACK),
                            Some(future),
                        ),
                        ComponentResponse::Update(data) => {
                            (InteractionResponse::UpdateMessage(data), None)
                        }
                        ComponentResponse::DeferredUpdate(future) => {
                            (InteractionResponse::DeferredUpdateMessage, Some(future))
                        }
                    }
                } else {
                    (
                        InteractionResponse::ChannelMessageWithSource(
                            "Error: no message component handler registered"
                                .to_string()
                                .into_callback_data(),
                        ),
                        None,
                    )
                };

                Response {
                    response,
                    future,
                    id: interaction.id,
                    token: interaction.token,
                }
            }
            _ => todo!(),
        }
    }

    #[cfg(any(feature = "gateway", feature = "webhook"))]
    async fn run_deferred(
        http: &Client,
        future: DeferredFuture,
        token: String,
    ) -> Result<(), Error> {
        let callback = future.await;

        let mut builder = http
            .update_interaction_original(&token)?
            .content(callback.content.as_deref())?
            .embeds(Some(&callback.embeds))?;

        if let Some(allowed_mentions) = callback.allowed_mentions {
            builder = builder.allowed_mentions(allowed_mentions);
        }

        builder.exec().await?;

        Ok(())
    }

    /// Handle an INTERACTION_CREATE event from the Discord Gateway, automatically sending the response over HTTP.
    ///
    /// Requires the `gateway` feature to be enabled.
    #[cfg(feature = "gateway")]
    pub async fn handle_event(
        &self,
        event: twilight_model::gateway::payload::InteractionCreate,
    ) -> Result<(), Error> {
        let response = self.handle(event.0);

        self.http
            .interaction_callback(response.id, &response.token, &response.response)
            .exec()
            .await?;

        if let Some(future) = response.future {
            Self::run_deferred(&self.http, future, response.token).await?;
        }

        Ok(())
    }

    #[cfg(feature = "webhook")]
    pub fn handle_request(
        &self,
        request: http::Request<&[u8]>,
        pub_key: &ed25519_dalek::PublicKey,
    ) -> Result<
        (
            http::Response<Vec<u8>>,
            Option<impl Future<Output = Result<(), Error>> + Send>,
        ),
        Error,
    > {
        use http::header::CONTENT_TYPE;
        use http::Response;
        use http::StatusCode;

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

        let response = self.handle(interaction);
        let token = response.token;

        let json = serde_json::to_vec(&response.response)?;

        Ok((
            Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, "application/json")
                .body(json)
                // If this is going to fail, it will always fail.
                .unwrap(),
            response.future.map(|future| {
                let http = self.http.clone();
                async move { Self::run_deferred(&http, future, token).await }
            }),
        ))
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
    global_commands: Vec<(&'static str, CommandDecl)>,
    guild_commands: HashMap<GuildId, Vec<(&'static str, CommandDecl)>>,
    component_handler: Option<
        Box<dyn Fn(Message, MessageComponentInteractionData) -> ComponentResponse + Send + Sync>,
    >,
    http: Client,
}

impl HandlerBuilder {
    pub fn global_command<T: Into<CommandDecl>>(mut self, name: &'static str, command: T) -> Self {
        self.global_commands.push((name, command.into()));
        self
    }

    pub fn guild_command<T: Into<CommandDecl>>(
        mut self,
        guild_id: GuildId,
        name: &'static str,
        command: T,
    ) -> Self {
        let guild_commands = self.guild_commands.entry(guild_id).or_insert_with(Vec::new);
        guild_commands.push((name, command.into()));
        self
    }

    pub fn component_handler<
        F: Fn(Message, MessageComponentInteractionData) -> ComponentResponse + Send + Sync + 'static,
    >(
        mut self,
        handler: F,
    ) -> Self {
        self.component_handler = Some(Box::new(handler));
        self
    }

    /// Registers the slash commands with Discord and returns the `Handler` to handle them.
    pub async fn build(self) -> Result<Handler, Error> {
        let mut command_handlers = Vec::new();

        // TODO: do this in parallel with the guild commands.
        let response = self
            .http
            .set_global_commands(
                &self
                    .global_commands
                    .iter()
                    .map(|(name, command)| command.description(name.to_string()))
                    .collect::<Vec<_>>(),
            )?
            .exec()
            .await?
            .models()
            .await?;

        for (command, description) in self
            .global_commands
            .into_iter()
            .map(|(_, command)| command)
            .zip(response.into_iter())
        {
            command_handlers.push((description.id.unwrap(), command.into()))
        }

        for (guild_id, commands) in self.guild_commands.into_iter() {
            let response = self
                .http
                .set_guild_commands(
                    guild_id,
                    &commands
                        .iter()
                        .map(|(name, command)| command.description(name.to_string()))
                        .collect::<Vec<_>>(),
                )?
                .exec()
                .await?
                .models()
                .await?;

            for (command, description) in commands
                .into_iter()
                .map(|(_, command)| command)
                .zip(response.into_iter())
            {
                command_handlers.push((description.id.unwrap(), command.into()))
            }
        }

        Ok(Handler {
            http: self.http,
            command_handlers,
            component_handler: self.component_handler,
        })
    }
}
