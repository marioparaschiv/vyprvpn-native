use std::fmt;
use std::io;

use interprocess::os::windows::security_descriptor::SecurityDescriptor;
use widestring::U16CString;

/// A string SID such as `S-1-5-21-...`, validated so it cannot inject extra ACEs into an SDDL string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sid(String);

#[derive(Debug, thiserror::Error)]
#[error("{0:?} is not a SID of the form S-1-<authority>-<subauthority>...")]
pub struct InvalidSid(String);

#[derive(Debug, thiserror::Error)]
pub enum CurrentSidError {
    #[error("finding the current user failed: {0}")]
    Lookup(#[from] io::Error),
    #[error(transparent)]
    Invalid(#[from] InvalidSid),
}

impl Sid {
    /// The user this process runs as.
    pub fn current() -> Result<Self, CurrentSidError> {
        Ok(Self::parse(&ipc::current_user_sid()?)?)
    }

    pub fn parse(text: &str) -> Result<Self, InvalidSid> {
        let numbers = text
            .strip_prefix("S-1-")
            .map(|rest| rest.split('-').collect::<Vec<_>>());
        let valid = numbers.is_some_and(|numbers| {
            numbers.len() >= 2
                && numbers.iter().all(|number| {
                    !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
                })
        });

        if !valid {
            return Err(InvalidSid(text.to_owned()));
        }

        Ok(Self(text.to_owned()))
    }
}

impl fmt::Display for Sid {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// SYSTEM serves the pipe; the owner, the user who installed VyprVPN, connects to it.
pub fn pipe_security(owner: &Sid) -> io::Result<SecurityDescriptor> {
    let sddl = U16CString::from_str(format!("D:P(A;;GA;;;SY)(A;;GRGW;;;{owner})"))
        .map_err(io::Error::other)?;

    SecurityDescriptor::deserialize(&sddl)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_user_and_well_known_sids() {
        for sid in ["S-1-5-21-3623811015-3361044348-30300820-1013", "S-1-5-18"] {
            assert_eq!(Sid::parse(sid).unwrap().to_string(), sid);
        }
    }

    #[test]
    fn rejects_anything_that_could_alter_the_sddl() {
        for text in [
            "",
            "SY",
            "S-1-5",
            "S-1-5-",
            "S-1-5-18)(A;;GA;;;WD",
            "S-1-5-x",
            "s-1-5-18",
        ] {
            assert!(Sid::parse(text).is_err(), "{text}");
        }
    }
}
