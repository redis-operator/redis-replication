use std::io::{BufWriter, Error, Write};
use std::net::{Shutdown, TcpStream};
use std::result::Result;
use std::result::Result::Ok;

use byteorder::ReadBytesExt;

use crate::config::Config;
use crate::constants::*;

mod constants;
mod config;

pub trait RedisEventListener {
    fn open(&mut self) -> Result<(), Error>;

    fn close(&self);
}

// 用于监听Redis单点的事件
pub struct StandaloneEventListener {
    host: &'static str,
    port: i32,
    password: &'static str,
    config: Config,
    writer: Option<BufWriter<TcpStream>>,
}

impl StandaloneEventListener {
    fn connect(&mut self) -> Result<(), Error> {
        let addr = format!("{}:{}", self.host, self.port);
        println!("connecting to {}", addr);
        let stream = TcpStream::connect(addr)?;
        println!("connected to server!");
        self.writer = Option::Some(BufWriter::new(stream));
        Ok(())
    }

    fn auth(&mut self) -> Result<(), Error> {
        if !self.password.is_empty() {
            self.send("AUTH".as_bytes(), &[self.password.as_bytes()])?;
            let resp = self.response()?;
            if resp != b"OK" {
                // TODO return error
                let resp = String::from_utf8(resp).unwrap();
                panic!("{} ", resp);
            }
        }
        Ok(())
    }

    fn send(&mut self, command: &[u8], args: &[&[u8]]) -> Result<(), Error> {
        let writer = self.writer.as_mut().unwrap();
        writer.write(&[STAR])?;
        let args_len = args.len() + 1;
        writer.write(args_len.to_string().as_bytes())?;
        writer.write(&[CR, LF, DOLLAR])?;
        writer.write(command.len().to_string().as_bytes())?;
        writer.write(&[CR, LF])?;
        writer.write(command)?;
        writer.write(&[CR, LF])?;
        for arg in args {
            writer.write(&[DOLLAR])?;
            writer.write(arg.len().to_string().as_bytes())?;
            writer.write(&[CR, LF])?;
            writer.write(arg)?;
            writer.write(&[CR, LF])?;
        }
        writer.flush()
    }

    fn response(&mut self) -> Result<Vec<u8>, Error> {
        let writer = self.writer.as_mut().unwrap();
        let socket = writer.get_mut();
        let byte = socket.read_u8()?;
        match byte {
            PLUS | MINUS => {
                let mut bytes = vec![];
                loop {
                    let byte = socket.read_u8()?;
                    if byte != CR {
                        bytes.push(byte);
                    } else {
                        break;
                    }
                }
                let byte = socket.read_u8()?;
                if byte == LF {
                    return Result::Ok(bytes);
                } else {
                    panic!("Expect LF after CR");
                }
            }
            _ => {
                Ok(Vec::default())
            }
        }
    }
}

impl RedisEventListener for StandaloneEventListener {
    fn open(&mut self) -> Result<(), Error> {
        self.connect()?;
        self.auth()?;
        Ok(())
    }

    fn close(&self) {
        let option = self.writer.as_ref();
        if self.writer.is_some() {
            println!("close connection with server...");
            option.unwrap().get_ref().shutdown(Shutdown::Both).unwrap();
        }
    }
}

pub fn new(host: &'static str, port: i32, password: &'static str) -> StandaloneEventListener {
    StandaloneEventListener {
        host,
        port,
        password,
        config: config::default(),
        writer: Option::None,
    }
}

// 测试用例
#[cfg(test)]
mod tests {
    use crate::{new, RedisEventListener};

    #[test]
    fn open() {
        let mut redis_listener = new("localhost", 6379, "123");
        if let Err(error) = redis_listener.open() {
            panic!("couldn't connect to server: {}", error)
        }
        redis_listener.close();
    }
}
