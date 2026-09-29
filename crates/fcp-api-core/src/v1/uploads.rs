//! The multipart forms the upload routes read, as the OpenAPI contract
//! describes them (docs/DEVELOPER_SURFACE.md §3). Never built: the handlers
//! read the fields as they stream; these name the fields and their types.

#![allow(dead_code)]

/// `POST /me/profile/avatar`.
#[derive(utoipa::ToSchema)]
pub struct AvatarUpload {
    /// The picture: PNG, JPEG, GIF or WebP. The field may also be named
    /// `avatar`.
    #[schema(value_type = String, format = Binary)]
    file: Vec<u8>,
}

/// `POST /me/voicemail/greetings`.
#[derive(utoipa::ToSchema)]
pub struct MyGreetingUpload {
    /// `unavailable`, `busy`, `name` or `temporary`.
    greeting_type: String,
    /// The recording: a WAV file.
    #[schema(value_type = String, format = Binary)]
    audio: Vec<u8>,
    /// A temporary greeting's lifetime, in hours.
    expires_in: Option<u64>,
}

/// `POST /voicemail/mailboxes/:id/greetings`.
#[derive(utoipa::ToSchema)]
pub struct GreetingUpload {
    /// `unavailable`, `busy`, `name` or `temporary`.
    greeting_type: String,
    /// The recording: a WAV file.
    #[schema(value_type = String, format = Binary)]
    audio: Vec<u8>,
}

/// `POST /prompts/:id/audio`.
#[derive(utoipa::ToSchema)]
pub struct PromptAudioUpload {
    /// The prompt's audio: a WAV file.
    #[schema(value_type = String, format = Binary)]
    audio: Vec<u8>,
}
