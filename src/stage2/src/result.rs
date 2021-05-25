use simple_error::SimpleError;

pub type Result<T> = std::result::Result<T, SimpleError>;
