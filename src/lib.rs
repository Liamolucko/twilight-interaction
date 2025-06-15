use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;

use thiserror::Error;
use twilight_http::request::application::interaction::update_original_response::UpdateOriginalResponseError;
use twilight_http::request::application::InteractionError;
use twilight_http::response::DeserializeBodyError;
use twilight_model::application::callback::CallbackData;
use twilight_model::application::callback::InteractionResponse;
use twilight_model::application::command::Command;
use twilight_model::application::command::CommandOption;
use twilight_model::application::command::CommandType;
use twilight_model::application::interaction::application_command::CommandDataOption;
use twilight_model::application::interaction::application_command::CommandInteractionDataResolved;
use twilight_model::channel::message::MessageFlags;
use twilight_model::channel::Message;
use twilight_model::id::InteractionId;
use twilight_model::user::User;

pub use twilight_interaction_macros::slash_command;
// Only show the trait in docs, not the derive macro.
#[doc(hidden)]
pub use twilight_interaction_macros::Choices;

mod context;
mod handler;
mod option_types;

pub use context::*;
pub use handler::*;
pub use option_types::*;

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
    /// The actual `InteractionResponse` to return to Discord.
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

pub(crate) type SlashHandlerFn = Box<
    dyn Fn(
            Context,
            Vec<CommandDataOption>,
            Option<CommandInteractionDataResolved>,
        ) -> Result<(InteractionResponse, Option<DeferredFuture>), String>
        + Send
        + Sync,
>;

pub(crate) type MessageHandlerFn =
    Box<dyn Fn(Context, Message) -> (InteractionResponse, Option<DeferredFuture>) + Send + Sync>;

pub(crate) type UserHandlerFn =
    Box<dyn Fn(Context, User) -> (InteractionResponse, Option<DeferredFuture>) + Send + Sync>;

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

impl<R: Into<InteractionResponse> + 'static> From<fn(Context, Message) -> R> for CommandDecl {
    fn from(func: fn(Context, Message) -> R) -> Self {
        CommandDecl::Message {
            handler: Box::new(move |context, message| {
                let response = func(context, message).into();
                match response {
                    InteractionResponse::Immediate(response) => (
                        InteractionResponse::ChannelMessageWithSource(response),
                        None,
                    ),
                    InteractionResponse::Deferred { ephemeral, future } => (
                        InteractionResponse::DeferredChannelMessageWithSource(CallbackData {
                            flags: Some(if ephemeral {
                                MessageFlags::EPHEMERAL
                            } else {
                                MessageFlags::empty()
                            }),
                            ..EMPTY_CALLBACK
                        }),
                        Some(future),
                    ),
                }
            }),
        }
    }
}

impl<R: Into<Response> + 'static> From<fn(Context, User) -> R> for CommandDecl {
    fn from(func: fn(Context, User) -> R) -> Self {
        CommandDecl::User {
            handler: Box::new(move |context, user| {
                let response = func(context, user).into();
                match response {
                    Response::Immediate(response) => (
                        InteractionResponse::ChannelMessageWithSource(response),
                        None,
                    ),
                    Response::Deferred { ephemeral, future } => (
                        InteractionResponse::DeferredChannelMessageWithSource(CallbackData {
                            flags: Some(if ephemeral {
                                MessageFlags::EPHEMERAL
                            } else {
                                MessageFlags::empty()
                            }),
                            ..EMPTY_CALLBACK
                        }),
                        Some(future),
                    ),
                }
            }),
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
