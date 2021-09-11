use std::collections::HashMap;

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use quote::ToTokens;
use syn::parse::Parse;
use syn::parse_macro_input;
use syn::spanned::Spanned;
use syn::AttributeArgs;
use syn::FnArg;
use syn::Ident;
use syn::ItemEnum;
use syn::ItemFn;
use syn::Lit;
use syn::LitStr;
use syn::Meta;
use syn::NestedMeta;
use syn::Pat;
use syn::ReturnType;
use syn::Token;

/// A thing representing the parameters for an attribute of the form #[foo = "bar"].
/// Used for parsing #[name = ""] and #[doc = ""]
struct EqStr {
    str: LitStr,
}

impl Parse for EqStr {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let _: Token![=] = input.parse()?;
        Ok(Self {
            str: input.parse()?,
        })
    }
}

// rustdoc complains about `twilight_model` not existing since this crate doesn't actually link to it,
// but this should only really be viewed in the docs for `twilight_interaction` anyway.
#[allow(rustdoc::broken_intra_doc_links)]
/// Declares a function usable as a slash command.
///
/// This allows the function to be passed to [`Handler`],
/// which will then a slash command with the correct name, types and arguments,
/// and use it to handle that command.
///
/// A `description` parameter needs to be passed to the macro,
/// to provide the description which Discord will display.
///
/// The function needs to return either a [`String`], in most cases,
/// or a [`CallbackData`] to set more advanced options.
///
/// ```no_run
/// use twilight_interaction::{slash_command, Handler};
///
/// #[slash_command(description("Prints 'Hello!'"))]
/// fn greet() -> String {
///     "Hello!".to_string()
/// }
///
/// # async {
/// // This is needed to register the slash command.
/// let http_client = twilight_http::Client::new("my_token".to_string())
///
/// let handler = Handler::new(http_client)
///     .global_command(greet::describe())
///     .build()
///     .await
///     .unwrap();
///
/// // Now we can use `handler` to handle incoming commands!
/// # };
/// ```
///
/// [`Handler`]: struct.Handler.html
/// [`CallbackData`]: ::twilight_model::application::callback::CallbackData
#[proc_macro_attribute]
pub fn slash_command(args: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as AttributeArgs);
    let item = parse_macro_input!(item as ItemFn);

    let mut description = None;
    let mut opt_descriptions = HashMap::new();
    let mut renames = HashMap::new();

    for arg in args {
        match &arg {
            NestedMeta::Meta(meta) => match meta {
                Meta::List(list) => {
                    if list.path.is_ident("description") {
                        for meta in &list.nested {
                            match meta {
                                NestedMeta::Lit(lit) => match lit {
                                    Lit::Str(str) => description = Some(str.value()),
                                    _ => {
                                        return syn::Error::new_spanned(
                                            lit,
                                            "Description must be a string literal",
                                        )
                                        .into_compile_error()
                                        .into()
                                    }
                                },
                                NestedMeta::Meta(meta) => match meta {
                                    Meta::NameValue(name_value) => {
                                        if let Some(ident) = name_value.path.get_ident() {
                                            opt_descriptions.insert(
                                                ident.clone(),
                                                match &name_value.lit {
                                                    Lit::Str(str) => str.value(),
                                                    lit => {
                                                        return syn::Error::new_spanned(
                                                            lit,
                                                            "Description must be a string literal",
                                                        )
                                                        .into_compile_error()
                                                        .into()
                                                    }
                                                },
                                            );
                                        } else {
                                            return syn::Error::new_spanned(
                                                &name_value.path,
                                                "Option must be ident",
                                            )
                                            .into_compile_error()
                                            .into();
                                        }
                                    }
                                    _ => {
                                        return syn::Error::new_spanned(meta, "Unexpected argument")
                                            .into_compile_error()
                                            .into()
                                    }
                                },
                            }
                        }
                    } else if list.path.is_ident("rename") {
                        for meta in &list.nested {
                            match meta {
                                NestedMeta::Meta(meta) => match meta {
                                    Meta::NameValue(name_value) => {
                                        if let Some(ident) = name_value.path.get_ident() {
                                            renames.insert(
                                                ident.clone(),
                                                match &name_value.lit {
                                                    Lit::Str(lit) => lit.clone(),
                                                    lit => {
                                                        return syn::Error::new_spanned(
                                                            lit,
                                                            "The new name must be a string literal",
                                                        )
                                                        .into_compile_error()
                                                        .into()
                                                    }
                                                },
                                            );
                                        } else {
                                            return syn::Error::new_spanned(
                                                &name_value.path,
                                                "The option name must be an ident",
                                            )
                                            .into_compile_error()
                                            .into();
                                        }
                                    }
                                    _ => {
                                        return syn::Error::new_spanned(meta, "Options to `rename` must be of the form `ident = \"name\"`")
                                            .into_compile_error()
                                            .into()
                                    }
                                },
                                _ =>    return syn::Error::new_spanned(meta, "Options to `rename` must be of the form `ident = \"name\"`")
                                .into_compile_error()
                                .into()
                            }
                        }
                    } else {
                        return syn::Error::new_spanned(list, "Unexpected argument")
                            .into_compile_error()
                            .into();
                    }
                }
                _ => {
                    return syn::Error::new_spanned(meta, "Unexpected argument")
                        .into_compile_error()
                        .into()
                }
            },
            NestedMeta::Lit(lit) => {
                return syn::Error::new_spanned(lit, "Unexpected argument")
                    .into_compile_error()
                    .into()
            }
        }
    }

    // These aren't particularly intuitive variable names because they're for use in `quote!`.
    let mut opt_type = Vec::new();
    let mut opt_name = Vec::new();
    let mut opt_description = Vec::new();
    // `opt_name`, but modified so that it definitely won't conflict with any of our internal variable names.
    let mut opt_ident = Vec::new();

    for arg in &item.sig.inputs {
        match arg {
            FnArg::Receiver(_) => {
                return syn::Error::new_spanned(
                    arg,
                    "Slash commands cannot have receiver arguments (`self`)",
                )
                .into_compile_error()
                .into()
            }
            FnArg::Typed(arg) => {
                opt_type.push(&*arg.ty);

                match &*arg.pat {
                    Pat::Ident(ident) => {
                        match opt_descriptions.remove(&ident.ident) {
                            Some(description) => opt_description.push(description),
                            None => {
                                return syn::Error::new_spanned(
                                    arg,
                                    format!("Missing description for `{}`", ident.ident),
                                )
                                .into_compile_error()
                                .into()
                            }
                        }

                        let name = match renames.remove(&ident.ident) {
                            Some(name) => name,
                            None => {
                                // Slash command argument names are kebab-case, whereas Rust argument names are snake_case.
                                // So, replace the underscores with dashes to translate.
                                LitStr::new(
                                    &ident.ident.to_string().replace('_', "-"),
                                    ident.span(),
                                )
                            }
                        };

                        // Validate the name
                        for char in name.value().chars() {
                            match char {
                                // Lowercase letters and dashes are allowed.
                                'a'..='z' | '-' => {},
                                // Any other characters are invalid for a slash command argument name.
                                _ => return syn::Error::new_spanned(name, "Argument names must be kebab-case (or snake_case, when written as an identifier)").into_compile_error().into(),
                            }
                        }
                        opt_name.push(name);
                        opt_ident.push(Ident::new(&(ident.ident.to_string() + "_"), ident.span()));
                    }
                    pat => {
                        return syn::Error::new_spanned(pat, "Only plain idents are supported.")
                            .into_compile_error()
                            .into()
                    }
                }
            }
        }
    }

    let description = if let Some(description) = description {
        LitStr::new(&description, Span::call_site())
    } else {
        return syn::Error::new(Span::call_site(), "Missing description")
            .into_compile_error()
            .into();
    };

    let output = match item.sig.output {
        ReturnType::Default => return syn::Error::new_spanned(item.sig, "Slash commands cannot return nothing.\nThey must either return a `String` or a `CallbackData`.").into_compile_error().into(),
        ReturnType::Type(_, ref ty) => ty.as_ref(),
    };

    let fn_name = &item.sig.ident;
    let name = LitStr::new(&fn_name.to_string().replace('_', "-"), fn_name.span());

    let gen_fn_name = Ident::new(&format!("__{}_describe", fn_name), fn_name.span());

    let convert_res = if item.sig.asyncness.is_some() {
        quote! {
            let fut = Box::pin(async move {
                <#output as IntoCallbackData>::into_callback_data(res.await)
            });

            Ok((InteractionResponse::DeferredChannelMessageWithSource(EMPTY_CALLBACK), Some(fut)))
        }
    } else {
        quote! {
            let res = <#output as IntoCallbackData>::into_callback_data(res);

            Ok((InteractionResponse::ChannelMessageWithSource(res), None))
        }
    };

    let mut tokens = item.to_token_stream();

    tokens.extend(quote! {
        // This needs to be in the same scope as the original function so that all the paths to the argument types stay correct.
        #[doc(hidden)]
        pub fn #gen_fn_name() -> ::twilight_interaction::CommandDecl {
            use ::std::boxed::Box;
            use ::std::convert::From;
            use ::std::option::Option::*;
            use ::std::primitive::str;
            use ::std::result::Result::*;
            use ::std::string::String;
            use ::std::vec;

            use ::twilight_model::application::callback::CallbackData;
            use ::twilight_model::application::callback::InteractionResponse;
            use ::twilight_interaction::SlashCommandOption;
            use ::twilight_interaction::IntoCallbackData;

            /// An empty `CallbackData`, to use for the pointless field of `InteractionResponse::DeferredChannelMessageWithSource`.
            const EMPTY_CALLBACK: CallbackData = CallbackData {
                allowed_mentions: None,
                components: None,
                content: None,
                embeds: vec![],
                flags: None,
                tts: None,
            };

            let options = vec![
                #(
                    <#opt_type as SlashCommandOption>::describe(<String as From<&str>>::from(#opt_name), <String as From<&str>>::from(#opt_description)),
                )*
            ];

            ::twilight_interaction::CommandDecl {
                name: #name,
                description: #description,
                options,
                handler: Box::new(|options, resolved| {
                    #(
                        let mut #opt_ident = None;
                    )*

                    for option in options {
                        #(
                            if option.name() == #opt_name {
                                #opt_ident = Some(option);
                            } else
                        )*
                        // If there are arguments, this will be an else block, otherwise it'll just be a regular block.
                        {
                            return Err(<String as From<&str>>::from(option.name()));
                        }
                    }

                    #(
                        let #opt_ident = <#opt_type as SlashCommandOption>::from_option(#opt_ident, resolved.as_ref()).ok_or(<String as From<&str>>::from(#opt_name))?;
                    )*

                    let res = #fn_name(#(#opt_ident),*);

                    #convert_res
                })
            }
        }

        // Create a module with the same name as the function which reexports our generated function, so that it's reexported along with the original function.
        #[doc(hidden)]
        pub mod #fn_name {
            pub use super::#gen_fn_name as describe;
        }
    });

    tokens.into()
}

#[proc_macro_derive(Choices, attributes(name))]
pub fn derive_choices(item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as ItemEnum);
    let name = item.ident;

    let mut next_discriminant = quote!(0);

    let mut names = Vec::with_capacity(item.variants.len());
    let mut values = Vec::with_capacity(item.variants.len());
    let mut display_names = Vec::with_capacity(item.variants.len());

    for variant in item.variants {
        let name_attr = variant
            .attrs
            .into_iter()
            .find(|attr| attr.path.is_ident("name"));

        let name = if let Some(attr) = name_attr {
            let tokens = attr.tokens.into();
            let args = parse_macro_input!(tokens as EqStr);
            args.str
        } else {
            LitStr::new(&variant.ident.to_string(), variant.ident.span())
        };
        let value = variant
            .discriminant
            // The highest enum discriminants can currently go is 64 bits,
            // and we only really care about having a unique value for each variant,
            // so just using an `as` cast here is fine.
            // (Also, Discord's integers can only go to 2**53 anyway. TODO add a check for that somehow)
            .map(|(_, value)| quote!(#value as ::std::primitive::i64))
            .unwrap_or(next_discriminant.clone());

        next_discriminant = quote!(::std::primitive::i64::wrapping_add(#value, 1));

        names.push(variant.ident);
        values.push(value);
        display_names.push(name);
    }

    (quote! {
        impl ::twilight_interaction::Choices for #name {
            const CHOICES: &'static [(&'static ::std::primitive::str, ::std::primitive::i64)] = &[
                #((#display_names, #values),)*
            ];

            fn from_discriminant(discriminant: ::std::primitive::i64) -> ::std::option::Option<Self> {
                #![allow(non_upper_case_globals)]
                #(
                    const #names: ::std::primitive::i64 = #values;
                )*
                match discriminant {
                    #(
                        #names => ::std::option::Option::Some(Self::#names),
                    )*
                    #[allow(unreachable_patterns)]
                    _ => ::std::option::Option::None,
                }
            }
        }
    })
    .into()
}
