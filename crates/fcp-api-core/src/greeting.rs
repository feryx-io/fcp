//! A mailbox's personalized greeting kept in line with its owner (#343):
//! made when a mailbox is made or its greeting choice changes, when its
//! name greeting is recorded or removed, and when its owner's name changes
//! — wherever that change comes from (an administrator, the user, SCIM,
//! single sign-on). The audio is made here, once, so a caller never waits
//! on TTS (`fcp_voicemail::personalized`).

use std::sync::Arc;

use fcp_core::storage::VoicemailStore;
use fcp_core::CredentialStore;
use fcp_core::{User, VoicemailMailbox};
use tracing::warn;

use crate::state::ApiState;

/// Whether a change to the user changes what their greeting says.
pub(crate) fn names_changed(before: &User, after: &User) -> bool {
    before.first_name != after.first_name
        || before.last_name != after.last_name
        || before.display_name != after.display_name
}

/// A display name that is only the first and last name is not kept: unset,
/// it reads as them and follows them when they change.
pub(crate) fn settle_display_name(user: &mut User) {
    user.display_name = settled_display_name(user);
}

/// The display name `user` keeps (`settle_display_name`).
pub(crate) fn settled_display_name(user: &User) -> Option<String> {
    let display = user.display_name.as_deref().map(str::trim)?;
    (!display.is_empty() && Some(display.to_string()) != user.full_name())
        .then(|| user.display_name.clone())
        .flatten()
}

/// One refresh of a mailbox's greeting at a time in this process: two
/// would speak the same sentence twice, or store the older last.
static REFRESHING: std::sync::LazyLock<dashmap::DashMap<String, Arc<tokio::sync::Mutex<()>>>> =
    std::sync::LazyLock::new(dashmap::DashMap::new);

/// Bring `mailbox`'s personalized greeting in line with `owner`; whether
/// the mailbox changed. A failure — no TTS, no greetings directory, the
/// TTS service down — is logged: callers then hear the standard greeting.
async fn refresh(state: &ApiState, mailbox: &mut VoicemailMailbox, owner: &User) -> bool {
    let Some(dir) = state.voicemail_greetings_dir.as_deref() else {
        if fcp_voicemail::personalized::source(mailbox, owner).is_some() {
            warn!(mailbox_id = %mailbox.id, "No voicemail greetings directory: the personalized greeting is not made");
        }
        return false;
    };
    match fcp_voicemail::personalized::refresh(state.tts_service.as_deref(), dir, mailbox, owner)
        .await
    {
        Ok(changed) => changed,
        Err(e) => {
            warn!(mailbox_id = %mailbox.id, error = %e, "The personalized greeting was not made");
            false
        }
    }
}

/// What the mailbox's personalized greeting should be made from, as kept.
fn wanted(mailbox: &VoicemailMailbox, owner: &User) -> Option<String> {
    fcp_voicemail::personalized::source(mailbox, owner)
        .map(|source| fcp_voicemail::personalized::made_from(mailbox, &source))
}

/// The mailbox's personalized greeting made again for its owner, as they
/// both are now, and stored. Speaking it takes a while, and the mailbox or
/// its owner may change meanwhile: the greeting is stored onto the mailbox
/// as it then is, only if it is still what that mailbox wants — otherwise
/// it is made again — so nothing written meanwhile is lost. The mailbox as
/// stored; `None` when it is gone.
pub(crate) async fn refresh_mailbox(
    state: &ApiState,
    mailbox_id: &str,
) -> Option<VoicemailMailbox> {
    let storage = state.storage.as_ref()?;
    let lock = REFRESHING
        .entry(mailbox_id.to_string())
        .or_default()
        .clone();
    let _one_at_a_time = lock.lock().await;
    let read = || async {
        let mailbox = storage.get_mailbox(mailbox_id).await.ok().flatten()?;
        let owner = storage
            .get_user_by_id(&mailbox.user_id)
            .await
            .ok()
            .flatten()?;
        Some((mailbox, owner))
    };
    let mut result = None;
    for _ in 0..3 {
        let (mailbox, owner) = read().await?;
        let mut made = mailbox.clone();
        if !refresh(state, &mut made, &owner).await {
            result = Some(mailbox);
            break;
        }
        let (now, now_owner) = read().await?;
        if wanted(&now, &now_owner) != wanted(&mailbox, &owner) {
            continue;
        }
        let greeting = made.greeting(fcp_core::GreetingType::Personalized).cloned();
        if let Err(e) = storage
            .set_greeting(mailbox_id, fcp_core::GreetingType::Personalized, greeting)
            .await
        {
            warn!(mailbox_id = %mailbox_id, error = %e, "The personalized greeting was not stored");
        }
        result = storage.get_mailbox(mailbox_id).await.ok().flatten();
        break;
    }
    drop(_one_at_a_time);
    REFRESHING.remove_if(mailbox_id, |_, l| Arc::strong_count(l) == 1);
    result
}

/// The user's mailbox's greeting, after their name changed: made in the
/// background, so a sign-in or a provisioner's write never waits on TTS.
pub(crate) fn user_renamed(state: &ApiState, before: &User, after: &User) {
    if !names_changed(before, after) {
        return;
    }
    let (state, owner) = (state.clone(), after.clone());
    tokio::spawn(async move {
        let Some(storage) = state.storage.as_ref() else {
            return;
        };
        match storage.get_mailbox_by_user(&owner.id).await {
            Ok(Some(mailbox)) => {
                refresh_mailbox(&state, &mailbox.id).await;
            }
            Ok(None) => {}
            Err(e) => warn!(user = %owner.username, error = %e, "The user's mailbox was not read"),
        }
    });
}
