use std::io;

use helpers::*;
use parse_helpers::*;

#[cfg_attr(test, derive(PartialEq))]
#[derive(Debug)]
pub struct NoopCommand<'a> {
    string: SmtpString<'a>,
}

impl<'a> NoopCommand<'a> {
    pub fn new(string: SmtpString) -> NoopCommand {
        NoopCommand { string }
    }

    pub fn string(&self) -> &SmtpString {
        &self.string
    }

    pub fn send_to(&self, w: &mut io::Write) -> io::Result<()> {
        w.write_all(b"NOOP ")?;
        w.write_all(self.string.as_bytes())?;
        w.write_all(b"\r\n")
    }

    pub fn take_ownership<'b>(self) -> NoopCommand<'b> {
        NoopCommand {
            string: self.string.take_ownership(),
        }
    }
}

named!(pub command_noop_args(&[u8]) -> NoopCommand, do_parse!(
    eat_spaces >>
    res: take_until!("\r\n") >>
    tag!("\r\n") >>
    (NoopCommand {
        string: res.into(),
    })
));

#[cfg(test)]
mod tests {
    use super::*;
    use nom::*;

    #[test]
    fn valid_command_noop_args() {
        let tests = vec![
            (
                &b" \t hello.world \t \r\n"[..],
                NoopCommand {
                    string: (&b"hello.world \t "[..]).into(),
                },
            ),
            (
                &b"\r\n"[..],
                NoopCommand {
                    string: (&b""[..]).into(),
                },
            ),
            (
                &b" \r\n"[..],
                NoopCommand {
                    string: (&b""[..]).into(),
                },
            ),
        ];
        for (s, r) in tests.into_iter() {
            assert_eq!(command_noop_args(s), IResult::Done(&b""[..], r));
        }
    }

    #[test]
    fn valid_send_to() {
        let mut v = Vec::new();
        NoopCommand::new((&b"useless string"[..]).into())
            .send_to(&mut v)
            .unwrap();
        assert_eq!(v, b"NOOP useless string\r\n");
    }
}
