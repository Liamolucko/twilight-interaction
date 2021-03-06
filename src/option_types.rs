use std::future::Future;
use std::pin::Pin;

use twilight_model::application::callback::CallbackData;
use twilight_model::application::callback::InteractionResponse;
use twilight_model::application::command::BaseCommandOptionData;
use twilight_model::application::command::ChoiceCommandOptionData;
use twilight_model::application::command::CommandOption;
use twilight_model::application::command::CommandOptionChoice;
use twilight_model::application::interaction::application_command::CommandDataOption;
use twilight_model::application::interaction::application_command::CommandInteractionDataResolved;
use twilight_model::application::interaction::application_command::InteractionChannel;
use twilight_model::guild::Role;
use twilight_model::id::ChannelId;
use twilight_model::id::RoleId;
use twilight_model::id::UserId;
use twilight_model::user::User;

use crate::DeferredFuture;
use crate::EMPTY_CALLBACK;

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
/// use twilight_interaction::Choices;
///
/// #[repr(i64)]
/// #[derive(Choices)]
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
pub trait Choices: Sized {
    const CHOICES: &'static [(&'static str, i64)];

    fn from_discriminant(discriminant: i64) -> Option<Self>;
}

/// A type which can be used as an option for a slash command.
pub trait SlashCommandOption: Sized {
    /// Generate a description for an option of this type with name `name` and description `description`.
    fn describe(name: String, description: String) -> CommandOption;
    /// Parse an instance of this type from an option given by Discord.
    /// `name` has already been checked; you only need to check if `value` is correct.
    /// Return `None` if something is wrong; the data is of the incorrect type or isn't present in `resolved`.
    fn from_option(
        data: Option<CommandDataOption>,
        resolved: Option<&CommandInteractionDataResolved>,
    ) -> Option<Self>;
}

impl SlashCommandOption for String {
    fn describe(name: String, description: String) -> CommandOption {
        CommandOption::String(ChoiceCommandOptionData {
            // TODO: make sure that this causes users to be able to enter anything, not nothing.
            choices: vec![],
            name,
            description,
            required: true,
        })
    }

    fn from_option(
        data: Option<CommandDataOption>,
        _: Option<&CommandInteractionDataResolved>,
    ) -> Option<Self> {
        match data {
            Some(CommandDataOption::String { value, .. }) => Some(value),
            _ => None,
        }
    }
}

impl SlashCommandOption for i64 {
    fn describe(name: String, description: String) -> CommandOption {
        CommandOption::Integer(ChoiceCommandOptionData {
            choices: vec![],
            name,
            description,
            required: true,
        })
    }

    fn from_option(
        data: Option<CommandDataOption>,
        _: Option<&CommandInteractionDataResolved>,
    ) -> Option<Self> {
        match data {
            Some(CommandDataOption::Integer { value, .. }) => Some(value),
            _ => None,
        }
    }
}

impl SlashCommandOption for bool {
    fn describe(name: String, description: String) -> CommandOption {
        CommandOption::Boolean(BaseCommandOptionData {
            name,
            description,
            required: true,
        })
    }

    fn from_option(
        data: Option<CommandDataOption>,
        _: Option<&CommandInteractionDataResolved>,
    ) -> Option<Self> {
        match data {
            Some(CommandDataOption::Boolean { value, .. }) => Some(value),
            _ => None,
        }
    }
}

impl SlashCommandOption for User {
    fn describe(name: String, description: String) -> CommandOption {
        CommandOption::User(BaseCommandOptionData {
            name,
            description,
            required: true,
        })
    }

    fn from_option(
        data: Option<CommandDataOption>,
        resolved: Option<&CommandInteractionDataResolved>,
    ) -> Option<Self> {
        match data {
            Some(CommandDataOption::String { value, .. }) => {
                let user_id = UserId::from(value.parse::<u64>().ok()?);

                resolved.and_then(|resolved| {
                    resolved
                        .users
                        .iter()
                        .find(|user| user.id == user_id)
                        .cloned()
                })
            }
            _ => None,
        }
    }
}

impl SlashCommandOption for InteractionChannel {
    fn describe(name: String, description: String) -> CommandOption {
        CommandOption::Channel(BaseCommandOptionData {
            name,
            description,
            required: true,
        })
    }

    fn from_option(
        data: Option<CommandDataOption>,
        resolved: Option<&CommandInteractionDataResolved>,
    ) -> Option<Self> {
        match data {
            Some(CommandDataOption::String { value, .. }) => {
                let channel_id = ChannelId::from(value.parse::<u64>().ok()?);

                resolved.and_then(|resolved| {
                    resolved
                        .channels
                        .iter()
                        .find(|channel| channel.id == channel_id)
                        .cloned()
                })
            }
            _ => None,
        }
    }
}

impl SlashCommandOption for Role {
    fn describe(name: String, description: String) -> CommandOption {
        CommandOption::Role(BaseCommandOptionData {
            name,
            description,
            required: true,
        })
    }

    fn from_option(
        data: Option<CommandDataOption>,
        resolved: Option<&CommandInteractionDataResolved>,
    ) -> Option<Self> {
        match data {
            Some(CommandDataOption::String { value, .. }) => {
                let role_id = RoleId::from(value.parse::<u64>().ok()?);

                resolved.and_then(|resolved| {
                    resolved
                        .roles
                        .iter()
                        .find(|role| role.id == role_id)
                        .cloned()
                })
            }
            _ => None,
        }
    }
}

impl SlashCommandOption for Mentionable {
    fn describe(name: String, description: String) -> CommandOption {
        CommandOption::Mentionable(BaseCommandOptionData {
            name,
            description,
            required: true,
        })
    }

    fn from_option(
        data: Option<CommandDataOption>,
        resolved: Option<&CommandInteractionDataResolved>,
    ) -> Option<Self> {
        match data {
            Some(CommandDataOption::String { value, .. }) => {
                let id = value.parse::<u64>().ok()?;

                resolved.and_then(|resolved| {
                    // First try to find a user matching the ID, otherwise look for a role.
                    resolved
                        .users
                        .iter()
                        .find(|user| user.id == UserId::from(id))
                        .cloned()
                        .map(Mentionable::User)
                        .or_else(|| {
                            resolved
                                .roles
                                .iter()
                                .find(|role| role.id == RoleId::from(id))
                                .cloned()
                                .map(Mentionable::Role)
                        })
                })
            }
            _ => None,
        }
    }
}

impl<T: Choices> SlashCommandOption for T {
    fn describe(name: String, description: String) -> CommandOption {
        CommandOption::Integer(ChoiceCommandOptionData {
            choices: Self::CHOICES
                .iter()
                .map(|&(name, value)| CommandOptionChoice::Int {
                    name: name.to_string(),
                    value,
                })
                .collect(),
            name,
            description,
            required: true,
        })
    }

    fn from_option(
        data: Option<CommandDataOption>,
        _: Option<&CommandInteractionDataResolved>,
    ) -> Option<Self> {
        match data {
            Some(CommandDataOption::Integer { value, .. }) => Self::from_discriminant(value),
            _ => None,
        }
    }
}

// FIXME: somehow disallow `Option<Option<Option<T>>>`.
impl<T: SlashCommandOption> SlashCommandOption for Option<T> {
    fn describe(name: String, description: String) -> CommandOption {
        let mut option = T::describe(name, description);
        match &mut option {
            CommandOption::SubCommand(data) | CommandOption::SubCommandGroup(data) => {
                data.required = false
            }
            CommandOption::String(data) | CommandOption::Integer(data) => data.required = false,
            CommandOption::Boolean(data)
            | CommandOption::User(data)
            | CommandOption::Channel(data)
            | CommandOption::Role(data)
            | CommandOption::Mentionable(data) => data.required = false,
        }
        option
    }

    fn from_option(
        data: Option<CommandDataOption>,
        resolved: Option<&CommandInteractionDataResolved>,
    ) -> Option<Self> {
        match data {
            Some(data) => T::from_option(Some(data), resolved).map(Some),
            None => Some(None),
        }
    }
}

/// A type which can be used as a response from a slash command.
pub trait IntoCallbackData {
    fn into_callback_data(self) -> CallbackData;
}

impl IntoCallbackData for CallbackData {
    fn into_callback_data(self) -> CallbackData {
        self
    }
}

// TODO: Ideally this'd be implemented for anything which implements `ToString`,
// but I can't because `CallbackData` might implement it in the future.
impl IntoCallbackData for String {
    fn into_callback_data(self) -> CallbackData {
        CallbackData {
            content: Some(self),
            allowed_mentions: None,
            embeds: vec![],
            flags: None,
            tts: None,
            components: None,
        }
    }
}

pub trait CommandResponse {
    fn into_interaction_response(self) -> (InteractionResponse, Option<DeferredFuture>);
}

impl<T> CommandResponse for T
where
    T: IntoCallbackData,
{
    fn into_interaction_response(self) -> (InteractionResponse, Option<DeferredFuture>) {
        (
            InteractionResponse::ChannelMessageWithSource(self.into_callback_data()),
            None,
        )
    }
}

impl<T> CommandResponse for Pin<Box<dyn Future<Output = T> + Send>>
where
    T: IntoCallbackData + 'static,
{
    fn into_interaction_response(self) -> (InteractionResponse, Option<DeferredFuture>) {
        (
            InteractionResponse::DeferredChannelMessageWithSource(EMPTY_CALLBACK),
            Some(Box::pin(async move { self.await.into_callback_data() })),
        )
    }
}
