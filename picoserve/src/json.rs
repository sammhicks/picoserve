/// A JSON encoded value.  
/// When serializing, the value might be serialized several times during sending,  
/// so the value must be serialized in the same way each time.  
/// When deserializing, only short strings can be unescaped.  
/// If you want to handle longed escaped strings, use [`JsonWithUnescapeBufferSize`](crate::extract::JsonWithUnescapeBufferSize).
pub struct Json<T>(pub T);
