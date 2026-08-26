/// Trait for converting a type into into a buffer.
pub trait Encode {
    /// The buffer type that holds the encoded data.
    type Buffer: AsRef<[u8]>;
    type Error;
    /// Converts the type into a byte buffer.
    fn encode(&self) -> Result<Self::Buffer, Self::Error>;
}
