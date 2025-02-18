//! This module holds typed parameters for various endpoint inputs.
//!
//! The purpose of this module is to define and manage the parameters that are used as inputs
//! for different endpoints in the web application. By using typed parameters, we can ensure
//! that the inputs are validated (by type) and correctly formatted before they are processed by the
//! application logic.
//!
//! Each parameter type is represented by a struct or enum, which can be serialized and
//! deserialized as needed. This approach helps to maintain a clear and consistent structure
//! for endpoint inputs, making the codebase easier to understand and maintain.
//
//! ```

pub(crate) mod jwt;
