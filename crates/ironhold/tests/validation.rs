//! Validation, end to end: an HTML form and a JSON API.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use ironhold::body::Body;
use ironhold::http::{Request, StatusCode, header};
use ironhold::prelude::*;
use ironhold::response::Response;
use ironhold::testing::TestClient;
use ironhold::{Config, Environment};
use serde::Deserialize;

#[derive(Deserialize)]
struct Subscribe {
    email: String,
    name: String,
}

impl Validate for Subscribe {
    fn rules(&self, v: &mut Validator) {
        v.check("email", &self.email).required().email();
        v.check("name", &self.name).max_chars(20);
    }
}

/// Only reachable with checked input.
fn subscribe(input: Valid<Subscribe>) -> String {
    format!("subscribed {}", input.email)
}

fn form(csrf: &CsrfToken, errors: &ValidationErrors, email: &str) -> Markup {
    html! {
        form method="post" action="/subscribe" {
            (csrf)
            input name="email" value=(email) aria-invalid=[errors.aria_invalid("email")]
                aria-describedby=[errors.described_by("email")];
            (errors.field("email"))
            input name="name";
            (errors.field("name"))
        }
    }
}

async fn page(csrf: CsrfToken) -> Markup {
    form(&csrf, &ValidationErrors::new(), "")
}

async fn submit(csrf: CsrfToken, Form(input): Form<Subscribe>) -> Response {
    let email = input.email.clone();
    match input.validate() {
        Ok(valid) => subscribe(valid).into_response(),
        Err(errors) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            form(&csrf, &errors, &email),
        )
            .into_response(),
    }
}

async fn api(Json(input): Json<Subscribe>) -> Response {
    match input.validate() {
        Ok(valid) => subscribe(valid).into_response(),
        Err(errors) => errors.into_response(),
    }
}

fn client() -> TestClient {
    TestClient::new(
        App::with_config(Config::new(Environment::Development))
            .route("/subscribe", get(page).post(submit))
            .route("/api/subscribe", post(api)),
    )
}

#[tokio::test]
async fn invalid_form_is_shown_again_with_field_errors() {
    let mut client = client();
    client.get("/subscribe").await;
    let response = client
        .post_form(
            "/subscribe",
            &[("email", "raj@"), ("name", "a name that is far too long")],
        )
        .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let html = response.text();
    assert!(
        html.contains(
            r#"<p class="field-error" id="email-error">Enter a valid email address.</p>"#
        )
    );
    assert!(
        html.contains(r#"<p class="field-error" id="name-error">Use at most 20 characters.</p>"#)
    );
    assert!(html.contains(r#"aria-invalid="true" aria-describedby="email-error""#));
    // The typed email is kept, so the user only fixes the mistake.
    assert!(html.contains(r#"value="raj@""#));
}

#[tokio::test]
async fn valid_form_goes_through() {
    let mut client = client();
    client.get("/subscribe").await;
    let response = client
        .post_form("/subscribe", &[("email", "raj@example.com"), ("name", "")])
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.text(), "subscribed raj@example.com");
}

#[tokio::test]
async fn json_api_gets_422_with_errors() {
    let mut client = client();
    let request = Request::post("/api/subscribe")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"email":"","name":""}"#))
        .unwrap();
    let response = client.send(request).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        response.text(),
        r#"{"errors":{"email":["This field is required."]}}"#
    );
}
