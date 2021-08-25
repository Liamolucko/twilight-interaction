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
use syn::LitBool;
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

#[proc_macro_attribute]
pub fn slash_command(args: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as AttributeArgs);
    let item = parse_macro_input!(item as ItemFn);

    let mut defer = false;

    let mut description = None;
    let mut opt_descriptions = HashMap::new();

    for arg in args {
        match &arg {
            NestedMeta::Meta(meta) => match meta {
                Meta::Path(path) => {
                    if path.is_ident("defer") {
                        if item.sig.asyncness.is_none() {
                            return syn::Error::new_spanned(
                                arg,
                                "Synchronous slash commands cannot be deferred",
                            )
                            .into_compile_error()
                            .into();
                        }
                        defer = true;
                    } else {
                        return syn::Error::new_spanned(meta, "Unexpected argument")
                            .into_compile_error()
                            .into();
                    }
                }
                Meta::List(list) => {
                    if !list.path.is_ident("description") {
                        return syn::Error::new_spanned(list, "Unexpected argument")
                            .into_compile_error()
                            .into();
                    }

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

    let defer = LitBool::new(defer, Span::call_site());

    // These aren't particularly intuitive variable names because they're for use in `quote!`.
    let mut opt_type = Vec::new();
    let mut opt_name = Vec::new();
    let mut opt_description = Vec::new();
    // `opt_name`, but modified so that it definitely won't conflict with any of our internal variable names.
    let mut opt_ident = Vec::new();

    for arg in &item.sig.inputs {
        match arg {
            FnArg::Receiver(_) => {
                return syn::Error::new_spanned(arg, "Slash commands cannot have receiver arguments (`self`)")
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

                        // Slash command argument names are kebab-case, whereas Rust argument names are snake_case.
                        // So, replace the underscores with dashes to translate.
                        let name = ident.ident.to_string().replace('_', "-");
                        // Validate the name
                        for char in name.chars() {
                            match char {
                                // Lowercase letters and underscores are allowed.
                                'a'..='z' | '_' => {},
                                // Any other characters are invalid for a slash command argument name.
                                _ => return syn::Error::new_spanned(ident, "Argument names must be snake_case (so they can map cleanly to Discord's kebab-case").into_compile_error().into(),
                            }
                        }
                        opt_name.push(LitStr::new(&name, ident.span()));
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
        return syn::Error::new(Span::call_site(), "Missing description").into_compile_error().into();
    };

    let output = match item.sig.output {
        ReturnType::Default => return syn::Error::new_spanned(item.sig, "Slash commands cannot return nothing.\nThey must either return a `String` or a `CallbackData`.").into_compile_error().into(),
        ReturnType::Type(_, ref ty) => ty.as_ref(),
    };

    let fn_name = &item.sig.ident;
    let name = LitStr::new(&fn_name.to_string(), fn_name.span());

    let convert_res = if item.sig.asyncness.is_some() {
        quote! {
            let fut = Box::pin(async move {
                <#output as ::twilight_slash_command::_macro_internal::InteractionResult>::into_callback_data(res.await)
            });

            Some(if #defer {
                CommandResponse::Deferred(fut)
            } else {
                CommandResponse::Async(fut)
            })
        }
    } else {
        quote! {
            let res = <#output as ::twilight_slash_command::_macro_internal::InteractionResult>::into_callback_data(res);

            Some(CommandResponse::Sync(res))
        }
    };

    let mut tokens = item.to_token_stream();

    tokens.extend(quote! {
        // Create a module with the same name as the function so that if the function is reexported,
        // all our stuff is reexported too and will still work.
        #[doc(hidden)]
        pub mod #fn_name {
            use ::twilight_slash_command::CommandDecl;
            use ::twilight_slash_command::CommandResponse;

            // Note: I don't think we have to worry about namespacing here, since it's it's own module,
            // so we can just use plain `Vec` and stuff.
            pub fn describe() -> CommandDecl {
                let options = vec![
                    #(
                        <#opt_type as ::twilight_slash_command::_macro_internal::InteractionOption>::describe(#opt_name.to_string(), #opt_description.to_string()),
                    )*
                ];

                CommandDecl {
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
                                }
                            ) else *
                        }

                        #(
                            let #opt_ident = <#opt_type as ::twilight_slash_command::_macro_internal::InteractionOption>::from_data(#opt_ident, resolved.as_ref())?;
                        )*

                        let res = super::#fn_name(#(#opt_ident),*);

                        #convert_res
                    })
                }
            }
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
            .map(|(_, value)| value.into_token_stream())
            .unwrap_or(next_discriminant.clone());

        next_discriminant = quote!(::std::primitive::i64::wrapping_add(#value, 1));

        names.push(name);
        values.push(value);
    }

    (quote! {
        impl ::twilight_slash_command::Choices for #name {
            const CHOICES: &'static [(&'static str, i64)] = &[
                #((#names, #values),)*
            ];
        }
    })
    .into()
}
