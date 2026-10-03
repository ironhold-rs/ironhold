//! Form validation with rules written in plain Rust.

use std::{collections::BTreeMap, fmt, ops::Deref};

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use maud::{Markup, html};
use serde::{Serialize, Serializer, ser::SerializeMap};

/// Input that can be validated.
///
/// ```
/// use ironhold_forms::{Validate, Validator};
///
/// #[derive(Debug)]
/// struct SignUp {
///     email: String,
///     name: String,
/// }
///
/// impl Validate for SignUp {
///     fn rules(&self, v: &mut Validator) {
///         v.check("email", &self.email).required().email();
///         v.check("name", &self.name).required().max_chars(100);
///     }
/// }
///
/// let input = SignUp { email: "not an email".into(), name: "Raj".into() };
/// let invalid = input.validate().unwrap_err();
/// assert_eq!(invalid.errors.first("email"), Some("Enter a valid email address."));
/// assert_eq!(invalid.errors.first("name"), None);
/// // The input comes back too, to show the form again with what was typed.
/// assert_eq!(invalid.input.email, "not an email");
/// ```
pub trait Validate: Sized {
    /// Declares the rules. Called by [`validate`](Validate::validate).
    fn rules(&self, v: &mut Validator);

    /// Checks every rule. Returns the input wrapped in [`Valid`] if all
    /// pass. Otherwise returns [`Invalid`]: the input as submitted, to show
    /// the form again, and each field's first problem.
    fn validate(self) -> Result<Valid<Self>, Invalid<Self>> {
        let mut v = Validator::default();
        self.rules(&mut v);
        if v.errors.is_empty() {
            Ok(Valid(self))
        } else {
            Err(Invalid {
                input: self,
                errors: v.errors,
            })
        }
    }
}

/// Input that failed validation.
///
/// In an HTML handler, show the form again from `input` with `errors` next
/// to each field. In a JSON API, return it as the response: a `422` with
/// the errors.
#[derive(Debug)]
pub struct Invalid<T> {
    /// The input as submitted.
    pub input: T,
    /// Each field's first problem.
    pub errors: ValidationErrors,
}

impl<T> IntoResponse for Invalid<T> {
    fn into_response(self) -> Response {
        self.errors.into_response()
    }
}

/// Input that passed validation.
///
/// The only way to get one is [`Validate::validate`], so a function that
/// takes `Valid<T>` can't be called with unchecked input.
#[derive(Debug, Clone)]
pub struct Valid<T>(T);

impl<T> Valid<T> {
    /// The validated input.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> Deref for Valid<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

/// Collects the problems found while checking rules.
#[derive(Debug, Default)]
pub struct Validator {
    errors: ValidationErrors,
}

impl Validator {
    /// Starts checking a text field. Rules other than
    /// [`required`](Check::required) pass when the value is empty, so
    /// optional fields only need checking when filled in.
    pub fn check<'a>(&'a mut self, field: &'static str, value: &'a str) -> Check<'a> {
        let failed = self.errors.has(field);
        Check {
            field,
            value,
            errors: &mut self.errors,
            failed,
            just_failed: false,
        }
    }

    /// Starts checking a number.
    pub fn check_number<N>(&mut self, field: &'static str, value: N) -> NumberCheck<'_, N>
    where
        N: PartialOrd + fmt::Display + Copy,
    {
        let failed = self.errors.has(field);
        NumberCheck {
            field,
            value,
            errors: &mut self.errors,
            failed,
            just_failed: false,
        }
    }

    /// Records a problem directly, for checks that don't fit a rule.
    pub fn error(&mut self, field: &'static str, message: impl Into<String>) {
        self.errors.add(field, message);
    }
}

/// The rules for one text field. Only the first failing rule is reported,
/// so each field shows one clear message.
pub struct Check<'a> {
    field: &'static str,
    value: &'a str,
    errors: &'a mut ValidationErrors,
    failed: bool,
    just_failed: bool,
}

impl Check<'_> {
    fn rule(mut self, passes: impl FnOnce(&str) -> bool, message: String) -> Self {
        self.just_failed = false;
        if !self.failed && !self.value.is_empty() && !passes(self.value) {
            self.errors.add(self.field, message);
            self.failed = true;
            self.just_failed = true;
        }
        self
    }

    /// The field must not be empty or only whitespace.
    pub fn required(mut self) -> Self {
        self.just_failed = false;
        if !self.failed && self.value.trim().is_empty() {
            self.errors.add(self.field, "This field is required.");
            self.failed = true;
            self.just_failed = true;
        }
        self
    }

    /// At least `min` characters.
    pub fn min_chars(self, min: usize) -> Self {
        self.rule(
            |v| v.chars().count() >= min,
            format!("Use at least {min} characters."),
        )
    }

    /// At most `max` characters.
    pub fn max_chars(self, max: usize) -> Self {
        self.rule(
            |v| v.chars().count() <= max,
            format!("Use at most {max} characters."),
        )
    }

    /// At most `max` bytes. Useful as a hard cap before expensive work such
    /// as password hashing.
    pub fn max_bytes(self, max: usize) -> Self {
        self.rule(|v| v.len() <= max, "This is too long.".to_owned())
    }

    /// Looks like an email address. This catches typos; only a confirmation
    /// email proves an address is real.
    pub fn email(self) -> Self {
        self.rule(looks_like_email, "Enter a valid email address.".to_owned())
    }

    /// An `http` or `https` URL.
    pub fn url(self) -> Self {
        self.rule(
            |v| {
                let rest = v
                    .strip_prefix("https://")
                    .or_else(|| v.strip_prefix("http://"));
                rest.is_some_and(|r| !r.is_empty() && !r.chars().any(char::is_whitespace))
            },
            "Enter a full web address starting with https://".to_owned(),
        )
    }

    /// One of `options`, for example a value from a `<select>`.
    pub fn one_of(self, options: &[&str]) -> Self {
        self.rule(
            |v| options.contains(&v),
            "Choose one of the options.".to_owned(),
        )
    }

    /// Equal to `other`, for example a password confirmation.
    pub fn equals(self, other: &str) -> Self {
        self.rule(|v| v == other, "This doesn't match.".to_owned())
    }

    /// A rule of your own: fails with `message` when `passes` is false.
    pub fn custom(self, passes: bool, message: impl Into<String>) -> Self {
        let message = message.into();
        self.rule(|_| passes, message)
    }

    /// Replaces the message of the rule just before this call, if it failed.
    ///
    /// ```
    /// # use ironhold_forms::Validator;
    /// # let mut v = Validator::default();
    /// v.check("email", "").required().message("Enter your email address.");
    /// ```
    pub fn message(self, message: impl Into<String>) -> Self {
        if self.just_failed {
            self.errors.replace_last(self.field, message.into());
        }
        self
    }
}

/// The rules for one number.
pub struct NumberCheck<'a, N> {
    field: &'static str,
    value: N,
    errors: &'a mut ValidationErrors,
    failed: bool,
    just_failed: bool,
}

impl<N: PartialOrd + fmt::Display + Copy> NumberCheck<'_, N> {
    fn rule(mut self, passes: bool, message: String) -> Self {
        self.just_failed = false;
        if !self.failed && !passes {
            self.errors.add(self.field, message);
            self.failed = true;
            self.just_failed = true;
        }
        self
    }

    /// At least `min`.
    pub fn min(self, min: N) -> Self {
        let passes = self.value >= min;
        self.rule(passes, format!("Must be at least {min}."))
    }

    /// At most `max`.
    pub fn max(self, max: N) -> Self {
        let passes = self.value <= max;
        self.rule(passes, format!("Must be at most {max}."))
    }

    /// Replaces the message of the rule just before this call, if it failed.
    pub fn message(self, message: impl Into<String>) -> Self {
        if self.just_failed {
            self.errors.replace_last(self.field, message.into());
        }
        self
    }
}

/// Validation problems, by field.
///
/// Render a field's problem next to its input with
/// [`field`](ValidationErrors::field). As a response, `ValidationErrors` is a `422` with JSON
/// like `{"errors": {"email": ["Enter a valid email address."]}}`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ValidationErrors {
    fields: BTreeMap<&'static str, Vec<String>>,
}

impl ValidationErrors {
    /// No problems.
    pub fn new() -> Self {
        Self::default()
    }

    /// A single problem, for example "this email is already registered"
    /// found after checking the database.
    pub fn single(field: &'static str, message: impl Into<String>) -> Self {
        let mut errors = Self::new();
        errors.add(field, message);
        errors
    }

    /// Adds a problem for `field`.
    pub fn add(&mut self, field: &'static str, message: impl Into<String>) {
        self.fields.entry(field).or_default().push(message.into());
    }

    /// Whether there are no problems.
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// Whether `field` has a problem.
    pub fn has(&self, field: &str) -> bool {
        self.fields.contains_key(field)
    }

    /// The first problem with `field`.
    pub fn first(&self, field: &str) -> Option<&str> {
        self.fields
            .get(field)
            .and_then(|messages| messages.first())
            .map(String::as_str)
    }

    /// Every field with problems, in name order.
    pub fn iter(&self) -> impl Iterator<Item = (&'static str, &[String])> {
        self.fields
            .iter()
            .map(|(field, messages)| (*field, messages.as_slice()))
    }

    /// The problem with `field` as `<p class="field-error" id="<field>-error">`,
    /// or nothing. Point the input at it with
    /// `aria-describedby=[errors.described_by("email")]` so screen readers
    /// announce it.
    pub fn field(&self, field: &str) -> Markup {
        match self.first(field) {
            Some(message) => html! {
                p.field-error id={ (field) "-error" } { (message) }
            },
            None => html! {},
        }
    }

    /// `Some("true")` when `field` has a problem, for `aria-invalid=[...]`.
    pub fn aria_invalid(&self, field: &str) -> Option<&'static str> {
        self.has(field).then_some("true")
    }

    /// `Some("<field>-error")` when `field` has a problem, for
    /// `aria-describedby=[...]`.
    pub fn described_by(&self, field: &str) -> Option<String> {
        self.has(field).then(|| format!("{field}-error"))
    }

    fn replace_last(&mut self, field: &'static str, message: String) {
        if let Some(last) = self.fields.get_mut(field).and_then(|m| m.last_mut()) {
            *last = message;
        }
    }
}

impl Serialize for ValidationErrors {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(1))?;
        map.serialize_entry("errors", &self.fields)?;
        map.end()
    }
}

impl IntoResponse for ValidationErrors {
    fn into_response(self) -> Response {
        (StatusCode::UNPROCESSABLE_ENTITY, Json(self)).into_response()
    }
}

pub(crate) fn looks_like_email(email: &str) -> bool {
    email.len() <= 254
        && !email.chars().any(char::is_whitespace)
        && email.split_once('@').is_some_and(|(user, domain)| {
            !user.is_empty()
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
                && !domain.contains('@')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Profile {
        email: String,
        name: String,
        website: String,
        age: u32,
        plan: String,
    }

    impl Validate for Profile {
        fn rules(&self, v: &mut Validator) {
            v.check("email", &self.email)
                .required()
                .message("Enter your email address.")
                .email();
            v.check("name", &self.name)
                .required()
                .min_chars(2)
                .max_chars(10);
            v.check("website", &self.website).url();
            v.check_number("age", self.age).min(13).max(150);
            v.check("plan", &self.plan)
                .required()
                .one_of(&["free", "pro"]);
        }
    }

    fn profile() -> Profile {
        Profile {
            email: "raj@example.com".into(),
            name: "Raj".into(),
            website: String::new(),
            age: 30,
            plan: "free".into(),
        }
    }

    #[test]
    fn valid_input_passes() {
        let valid = profile().validate().unwrap();
        assert_eq!(valid.name, "Raj");
    }

    #[test]
    fn each_field_reports_its_first_problem() {
        let errors = Profile {
            email: "  ".into(),
            name: "a".into(),
            website: "ftp://x".into(),
            age: 5,
            plan: "gold".into(),
        }
        .validate()
        .unwrap_err()
        .errors;
        assert_eq!(errors.first("email"), Some("Enter your email address."));
        assert_eq!(errors.first("name"), Some("Use at least 2 characters."));
        assert_eq!(
            errors.first("website"),
            Some("Enter a full web address starting with https://")
        );
        assert_eq!(errors.first("age"), Some("Must be at least 13."));
        assert_eq!(errors.first("plan"), Some("Choose one of the options."));
        assert!(errors.iter().all(|(_, messages)| messages.len() == 1));
    }

    #[test]
    fn message_only_replaces_the_rule_that_just_failed() {
        let errors = Profile {
            email: "not-an-email".into(),
            ..profile()
        }
        .validate()
        .unwrap_err()
        .errors;
        // `required` passed, so its custom message isn't used.
        assert_eq!(errors.first("email"), Some("Enter a valid email address."));
    }

    #[test]
    fn optional_fields_skip_rules_when_empty() {
        assert!(
            Profile {
                website: String::new(),
                ..profile()
            }
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn characters_not_bytes() {
        let errors = Profile {
            name: "\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}".into(),
            ..profile()
        }
        .validate();
        assert!(errors.is_ok(), "10 two-byte characters fit max_chars(10)");
    }

    #[test]
    fn email_check() {
        for good in ["a@b.co", "first.last+tag@example.co.uk"] {
            assert!(looks_like_email(good), "{good}");
        }
        for bad in [
            "", "a", "a@b", "@b.co", "a@.co", "a@b.", "a b@c.co", "a@b@c.co",
        ] {
            assert!(!looks_like_email(bad), "{bad}");
        }
    }

    #[test]
    fn field_markup_and_aria_helpers() {
        let errors = ValidationErrors::single("email", "Taken <already>");
        assert_eq!(
            errors.field("email").into_string(),
            r#"<p class="field-error" id="email-error">Taken &lt;already&gt;</p>"#
        );
        assert_eq!(errors.field("name").into_string(), "");
        assert_eq!(errors.aria_invalid("email"), Some("true"));
        assert_eq!(errors.described_by("email").as_deref(), Some("email-error"));
        assert_eq!(errors.aria_invalid("name"), None);
    }

    #[test]
    fn errors_serialize_for_apis() {
        let mut errors = ValidationErrors::single("email", "Enter a valid email address.");
        errors.add("name", "This field is required.");
        assert_eq!(
            serde_json::to_string(&errors).unwrap(),
            r#"{"errors":{"email":["Enter a valid email address."],"name":["This field is required."]}}"#
        );
        assert_eq!(
            errors.into_response().status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
}
