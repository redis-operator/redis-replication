/*!
用于监听Redis的写入操作，据此可以实现数据复制，监控等相关的应用。

# 原理

此crate实现了[Redis Replication协议]，在运行时，程序将以replica的身份连接到Redis，相当于Redis的一个副本。

所以，在程序连接上某个Redis之后，Redis会将它当前的所有数据以RDB的格式dump一份，dump完毕之后便发送过来，这个RDB中的每一条数据就对应一个[`Event`]`::RDB`事件。

在这之后，Redis接收到来自客户端的写入操作(即Redis命令)后，也会将这个写入操作传播给它的replica，每一个写入操作就对应一个[`Event`]`::AOF`事件。

# 示例

```rust,no_run
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::str::FromStr;
use std::rc::Rc;
use std::cell::RefCell;
use redis_event::listener;
use redis_event::config::Config;
use redis_event::{NoOpEventHandler, RedisListener};


let ip = IpAddr::from_str("127.0.0.1").unwrap();
let port = 6379;

let conf = Config {
    is_discard_rdb: false,            // 不跳过RDB
    is_aof: false,                    // 不处理AOF
    addr: SocketAddr::new(ip, port),
    password: String::new(),          // 密码为空
    repl_id: String::from("?"),       // replication id，若无此id，设置为?即可
    repl_offset: -1,                  // replication offset，若无此offset，设置为-1即可
    read_timeout: None,               // None，即读取永不超时
    write_timeout: None,              // None，即写入永不超时
};
let running = Arc::new(AtomicBool::new(true));
let mut redis_listener = listener::new(conf, running);
// 设置事件处理器
redis_listener.set_event_handler(Rc::new(RefCell::new(NoOpEventHandler{})));
// 启动程序
redis_listener.start()?;
```

[Redis Replication协议]: https://redis.io/topics/replication
[`Event`]: enum.Event.html
*/

use std::io::{Read, Result};

use crate::cmd::Command;
use crate::rdb::{Module, Object};

pub mod cmd;
pub mod config;
mod io;
mod iter;
pub mod listener;
mod lzf;
pub mod rdb;
mod resp;
mod tests;

/// Redis事件监听器的定义，所有类型的监听器都实现此接口
pub trait RedisListener {
    /// 开启事件监听
    fn start(&mut self) -> Result<()>;
}

/// Redis RDB 解析器定义
pub trait RDBParser {
    fn parse(
        &mut self,
        input: &mut dyn Read,
        length: i64,
        event_handler: &mut dyn EventHandler,
    ) -> Result<()>;
}

/// Redis事件
pub enum Event<'a> {
    /// RDB事件
    ///
    /// 当开启`RedisListener`之后，Redis会将此刻内存中的数据dump出来(以rdb的格式进行dump)，
    /// dump完毕之后的rdb数据便会发送给`RedisListener`，此rdb中的数据即对应此事件
    RDB(Object<'a>),
    /// AOF事件
    ///
    /// 在上面rdb数据处理完毕之后，客户端对Redis的数据写入操作将会发送给`RedisListener`，
    /// 此写入操作即对应此事件
    AOF(Command<'a>),
}

/// Redis事件处理器的定义，所有类型的处理器都必须实现此接口
pub trait EventHandler {
    fn handle(&mut self, event: Event);
}

/// No Operation处理器，对于接收到的事件，不做任何处理
pub struct NoOpEventHandler {}

impl EventHandler for NoOpEventHandler {
    fn handle(&mut self, _: Event) {}
}

/// Module Parser
pub trait ModuleParser {
    fn parse(
        &mut self,
        input: &mut dyn Read,
        module_name: &str,
        module_version: usize,
    ) -> Box<dyn Module>;
}

/// 转换为utf-8字符串，不验证正确性
fn to_string(bytes: Vec<u8>) -> String {
    return unsafe { String::from_utf8_unchecked(bytes) };
}
