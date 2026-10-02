use ironhold_core::Error;
use ironhold_session::{CsrfToken, Session};
use serde::Serialize;

/// Session key holding the logged-in user's id.
pub(crate) const USER_KEY: &str = "ironhold.auth.user";

/// Logs `user_id` in on this session.
///
/// The session gets a new id (so an attacker who planted a session id
/// before login can't use it afterwards) and a new CSRF token.
pub async fn login<Id: Serialize + Sync>(session: &Session, user_id: &Id) -> Result<(), Error> {
    session.cycle_id().await?;
    session.insert(USER_KEY, user_id).await?;
    CsrfToken::rotate(session).await?;
    Ok(())
}

/// Logs out by deleting the whole session, on the server and in the
/// browser.
pub async fn logout(session: &Session) -> Result<(), Error> {
    session.flush().await?;
    Ok(())
}
