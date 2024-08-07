/// A JSON encoded value. When serializing, the value might be serialized several times during sending, so the value must be serialized in the same way each time.
/// When values are deserialized, `UNESCAPE_BUFFER_SIZE` is the size of the temporary buffer used for unescaping strings.
pub struct Json<T, const UNESCAPE_BUFFER_SIZE: usize = 32>(pub T);
