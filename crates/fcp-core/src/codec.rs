/// Placeholder for media codec definitions.
///
/// The initial MVP focuses on call setup; codecs will be fleshed out in
/// later sprints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    Pcmu,
    Pcma,
    Opus,
}
