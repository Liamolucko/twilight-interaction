use std::env;
use std::sync::Arc;

use ed25519_dalek::PublicKey;
use ed25519_dalek::PUBLIC_KEY_LENGTH;
use hex::FromHex;
use http::Request;
use hyper::service::service_fn;
use hyper::Body;
use hyper::Server;
use tower::make::Shared;
use twilight_http::Client;
use twilight_slash_command::Handler;

#[path = "common/commands.rs"]
mod commands;

use commands::{all_the_args, default, frob, greet, random, rust_version};

#[tokio::main]
async fn main() {
    let token = env::var("TOKEN").expect("Missing discord bot token");
    let application_id = env::var("APP_ID")
        .expect("Missing application ID")
        .parse::<u64>()
        .unwrap()
        .into();
    let guild_id = env::var("GUILD_ID")
        .expect("Missing guild ID")
        .parse::<u64>()
        .unwrap()
        .into();

    let hex = env::var("PUBLIC_KEY").expect("Missing discord public key");
    let bytes: [u8; PUBLIC_KEY_LENGTH] =
        FromHex::from_hex(hex).expect("Public key was invalid hex");
    let public_key = PublicKey::from_bytes(&bytes).expect("Public key was invalid");

    let http = Client::new(token.clone());
    http.set_application_id(application_id);

    let handler = Handler::builder(http.clone())
        .guild_command(guild_id, all_the_args::describe())
        .guild_command(guild_id, default::describe())
        .guild_command(guild_id, frob::describe())
        .guild_command(guild_id, greet::describe())
        .guild_command(guild_id, random::describe())
        .guild_command(guild_id, rust_version::describe())
        .build()
        .await
        .unwrap();

    let handler = Arc::new(handler);

    // Local address to bind the service to.
    let addr = "0.0.0.0:8080".parse().unwrap();

    let service = service_fn(move |req| {
        let handler = Arc::clone(&handler);
        async move {
            // Convert from a hyper `Body` into a byte slice.
            let (parts, body) = req.into_parts();
            let bytes = hyper::body::to_bytes(body).await?;
            let req = Request::from_parts(parts, bytes.as_ref());

            // Get the response.
            let (res, fut) = handler.handle_request(req, &public_key).await?;

            // Run the deferred future, if any.
            if let Some(fut) = fut {
                tokio::spawn(fut);
            }

            // Convert the response into a hyper `Body`.
            Ok::<_, anyhow::Error>(res.map(Body::from))
        }
    });

    let make_service = Shared::new(service);

    let server = Server::bind(&addr).serve(make_service);

    // Start the server.
    server.await.unwrap();
}
