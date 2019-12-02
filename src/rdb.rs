use std::io::{Cursor, Error, ErrorKind, Read};

use byteorder::{BigEndian, LittleEndian, ReadBytesExt};

use crate::{CommandHandler, OBJ_HASH, OBJ_LIST, OBJ_SET, OBJ_STRING, OBJ_ZSET, RdbHandler};
use crate::iter::{IntSetIter, QuickListIter, SortedSetIter, StrValIter, ZipListIter, ZipMapIter};
use crate::rdb::Data::{Bytes, Empty};
use crate::reader::Reader;

// 回车换行，在redis响应中一般表示终结符，或用作分隔符以分隔数据
pub(crate) const CR: u8 = b'\r';
pub(crate) const LF: u8 = b'\n';
// 代表array响应
pub(crate) const STAR: u8 = b'*';
// 代表bulk string响应
pub(crate) const DOLLAR: u8 = b'$';
// 代表simple string响应
pub(crate) const PLUS: u8 = b'+';
// 代表error响应
pub(crate) const MINUS: u8 = b'-';
// 代表integer响应
pub(crate) const COLON: u8 = b':';

/// Map object types to RDB object types.
///
const RDB_TYPE_STRING: u8 = 0;
const RDB_TYPE_LIST: u8 = 1;
const RDB_TYPE_SET: u8 = 2;
const RDB_TYPE_ZSET: u8 = 3;
const RDB_TYPE_HASH: u8 = 4;
/// ZSET version 2 with doubles stored in binary.
const RDB_TYPE_ZSET_2: u8 = 5;
const RDB_TYPE_MODULE: u8 = 6;
/// Module value with annotations for parsing without
/// the generating module being loaded.
const RDB_TYPE_MODULE_2: u8 = 7;

/// Object types for encoded objects.
///
const RDB_TYPE_HASH_ZIPMAP: u8 = 9;
const RDB_TYPE_LIST_ZIPLIST: u8 = 10;
const RDB_TYPE_SET_INTSET: u8 = 11;
const RDB_TYPE_ZSET_ZIPLIST: u8 = 12;
const RDB_TYPE_HASH_ZIPLIST: u8 = 13;
const RDB_TYPE_LIST_QUICKLIST: u8 = 14;
const RDB_TYPE_STREAM_LISTPACKS: u8 = 15;

/// Special RDB opcodes
///
// Module auxiliary data.
const RDB_OPCODE_MODULE_AUX: u8 = 247;
// LRU idle time.
const RDB_OPCODE_IDLE: u8 = 248;
// LFU frequency.
const RDB_OPCODE_FREQ: u8 = 249;
// RDB aux field.
const RDB_OPCODE_AUX: u8 = 250;
// Hash table resize hint.
const RDB_OPCODE_RESIZEDB: u8 = 251;
// Expire time in milliseconds.
const RDB_OPCODE_EXPIRETIME_MS: u8 = 252;
// Old expire time in seconds.
const RDB_OPCODE_EXPIRETIME: u8 = 253;
// DB number of the following keys.
const RDB_OPCODE_SELECTDB: u8 = 254;
// End of the RDB file.
const RDB_OPCODE_EOF: u8 = 255;

const ZIP_INT_8BIT: u8 = 254;
const ZIP_INT_16BIT: u8 = 192;
const ZIP_INT_24BIT: u8 = 240;
const ZIP_INT_32BIT: u8 = 208;
const ZIP_INT_64BIT: u8 = 224;

// 用于包装redis的返回值
pub(crate) enum Data<B, V> {
    // 包装Vec<u8>
    Bytes(B),
    // 包装Vec<Vec<u8>>
    BytesVec(V),
    // 空返回
    Empty,
}

// 读取、解析rdb
pub(crate) fn parse(input: &mut Reader,
                    length: isize,
                    rdb_handlers: &Vec<Box<dyn RdbHandler>>,
                    cmd_handler: &Vec<Box<dyn CommandHandler>>) -> Result<Data<Vec<u8>, Vec<Vec<u8>>>, Error> {
    println!("rdb size: {} bytes", length);
    let mut bytes = vec![0; 5];
    // 开头5个字节: REDIS
    input.read_exact(&mut bytes)?;
    // 4个字节: rdb版本
    input.read_exact(&mut bytes[..=3])?;
    let rdb_version = String::from_utf8(bytes[..=3].to_vec()).unwrap();
    let rdb_version = rdb_version.parse::<isize>().unwrap();
    loop {
        let data_type = input.read_u8()?;
        match data_type {
            RDB_OPCODE_AUX => {
                let field_name = input.read_string()?;
                let field_val = input.read_string()?;
                let field_name = String::from_utf8(field_name).unwrap();
                let field_val = String::from_utf8(field_val).unwrap();
                println!("{}:{}", field_name, field_val);
            }
            RDB_OPCODE_SELECTDB => {
                let (db, _) = input.read_length()?;
                println!("db: {}", db);
            }
            RDB_OPCODE_RESIZEDB => {
                let (db, _) = input.read_length()?;
                println!("db total keys: {}", db);
                let (db, _) = input.read_length()?;
                println!("db expired keys: {}", db);
            }
            RDB_OPCODE_EXPIRETIME => {}
            RDB_OPCODE_EXPIRETIME_MS => {
                let expire_times = input.read_integer(8, false)?;
                let _type = input.read_u8()?;
                match _type {
                    RDB_OPCODE_FREQ => {
                        let frq = input.read_u8()?;
                        let value_type = input.read_u8()?;
                    }
                    RDB_OPCODE_IDLE => {}
                    _ => {}
                    // TODO
                }
            }
            RDB_OPCODE_FREQ => {
                // TODO
            }
            RDB_OPCODE_IDLE => {
                // TODO
            }
            RDB_OPCODE_MODULE_AUX => {
                // TODO
            }
            RDB_TYPE_MODULE => {
                // TODO
            }
            RDB_TYPE_MODULE_2 => {
                // TODO
            }
            RDB_TYPE_STREAM_LISTPACKS => {
                // TODO
            }
            RDB_TYPE_STRING => {
                let key = input.read_string()?;
                let mut iter = StrValIter { count: 1, input };
                rdb_handlers.iter().for_each(|handler|
                    handler.handle(&key, &mut iter, OBJ_STRING)
                );
            }
            RDB_TYPE_HASH_ZIPLIST | RDB_TYPE_ZSET_ZIPLIST | RDB_TYPE_LIST_ZIPLIST => {
                let key = input.read_string()?;
                let bytes = input.read_string()?;
                let cursor = &mut Cursor::new(bytes);
                // 跳过ZL_BYTES和ZL_TAIL
                cursor.set_position(8);
                let count = cursor.read_u16::<LittleEndian>()? as isize;
                let mut iter = ZipListIter { count, cursor };
                let _type;
                if data_type == RDB_TYPE_HASH_ZIPLIST {
                    _type = OBJ_HASH;
                } else if data_type == RDB_TYPE_ZSET_ZIPLIST {
                    _type = OBJ_ZSET;
                } else {
                    _type = OBJ_LIST;
                }
                rdb_handlers.iter().for_each(|handler|
                    handler.handle(&key, &mut iter, _type)
                );
            }
            RDB_TYPE_LIST_QUICKLIST => {
                let key = input.read_string()?;
                let (count, _) = input.read_length()?;
                let mut iter = QuickListIter { len: -1, count, input, cursor: Option::None };
                rdb_handlers.iter().for_each(|handler|
                    handler.handle(&key, &mut iter, OBJ_LIST)
                );
            }
            RDB_TYPE_LIST | RDB_TYPE_SET => {
                let key = input.read_string()?;
                let (count, _) = input.read_length()?;
                let mut iter = StrValIter { count, input };
                let _type;
                if data_type == RDB_TYPE_LIST {
                    _type = OBJ_LIST;
                } else {
                    _type = OBJ_SET;
                }
                rdb_handlers.iter().for_each(|handler|
                    handler.handle(&key, &mut iter, _type));
            }
            RDB_TYPE_ZSET => {
                let key = input.read_string()?;
                let (count, _) = input.read_length()?;
                let mut iter = SortedSetIter { count, v: 1, read_score: false, input };
                rdb_handlers.iter().for_each(|handler|
                    handler.handle(&key, &mut iter, OBJ_ZSET));
            }
            RDB_TYPE_ZSET_2 => {
                let key = input.read_string()?;
                let (count, _) = input.read_length()?;
                let mut iter = SortedSetIter { count, v: 2, read_score: false, input };
                rdb_handlers.iter().for_each(|handler|
                    handler.handle(&key, &mut iter, OBJ_ZSET));
            }
            RDB_TYPE_HASH => {
                let key = input.read_string()?;
                let (count, _) = input.read_length()?;
                let mut iter = StrValIter { count, input };
                rdb_handlers.iter().for_each(|handler|
                    handler.handle(&key, &mut iter, OBJ_HASH));
            }
            RDB_TYPE_HASH_ZIPMAP => {
                let key = input.read_string()?;
                let bytes = input.read_string()?;
                let cursor = &mut Cursor::new(&bytes);
                cursor.set_position(1);
                let mut iter = ZipMapIter { has_more: true, read_val: false, cursor };
                rdb_handlers.iter().for_each(|handler|
                    handler.handle(&key, &mut iter, OBJ_HASH));
            }
            RDB_TYPE_SET_INTSET => {
                let key = input.read_string()?;
                let bytes = input.read_string()?;
                let mut cursor = Cursor::new(&bytes);
                let encoding = cursor.read_i32::<LittleEndian>()?;
                let length = cursor.read_u32::<LittleEndian>()?;
                let mut iter = IntSetIter { encoding, count: length as isize, cursor: &mut cursor };
                rdb_handlers.iter().for_each(|handler|
                    handler.handle(&key, &mut iter, OBJ_SET));
            }
            RDB_OPCODE_EOF => {
                if rdb_version >= 5 {
                    input.read_integer(8, true)?;
                }
                break;
            }
            _ => break
        };
    };
    Ok(Empty)
}

// 当redis响应的数据是Bulk string时，使用此方法读取指定length的字节, 并返回
pub(crate) fn read_bytes(input: &mut Reader, length: isize, _: &Vec<Box<dyn RdbHandler>>, _: &Vec<Box<dyn CommandHandler>>) -> Result<Data<Vec<u8>, Vec<Vec<u8>>>, Error> {
    if length > 0 {
        let mut bytes = vec![0; length as usize];
        input.read_exact(&mut bytes)?;
        let end = &mut [0; 2];
        input.read_exact(end)?;
        if end == b"\r\n" {
            return Ok(Bytes(bytes));
        } else {
            let err = format!("Expect CRLF after bulk string, but got: {:?}", end);
            return Err(Error::new(ErrorKind::Other, err));
        }
    } else if length == 0 {
        // length == 0 代表空字符，后面还有CRLF
        input.read_exact(&mut [0; 2])?;
        return Ok(Empty);
    } else {
        // length < 0 代表null
        return Ok(Empty);
    }
}

pub(crate) fn read_zm_len(cursor: &mut Cursor<&Vec<u8>>) -> Result<usize, Error> {
    let len = cursor.read_u8()?;
    if len <= 253 {
        return Ok(len as usize);
    } else if len == 254 {
        let value = cursor.read_u32::<BigEndian>()?;
        return Ok(value as usize);
    }
    Ok(len as usize)
}

pub(crate) fn read_zip_list_entry(cursor: &mut Cursor<Vec<u8>>) -> Result<Vec<u8>, Error> {
    if cursor.read_u8()? >= 254 {
        cursor.read_u32::<LittleEndian>()?;
    }
    let flag = cursor.read_u8()?;
    match flag >> 6 {
        0 => {
            let length = flag & 0x3F;
            let mut buff = vec![0; length as usize];
            cursor.read_exact(&mut buff)?;
            return Ok(buff);
        }
        1 => {
            let next_byte = cursor.read_u8()?;
            let length = (((flag as u16) & 0x3F) << 8) | (next_byte as u16);
            let mut buff = vec![0; length as usize];
            cursor.read_exact(&mut buff)?;
            return Ok(buff);
        }
        2 => {
            let length = cursor.read_u32::<BigEndian>()?;
            let mut buff = vec![0; length as usize];
            cursor.read_exact(&mut buff)?;
            return Ok(buff);
        }
        _ => {}
    }
    match flag {
        ZIP_INT_8BIT => {
            let int = cursor.read_i8()?;
            return Ok(int.to_string().into_bytes());
        }
        ZIP_INT_16BIT => {
            let int = cursor.read_i16::<LittleEndian>()?;
            return Ok(int.to_string().into_bytes());
        }
        ZIP_INT_24BIT => {
            let int = cursor.read_i24::<LittleEndian>()?;
            return Ok(int.to_string().into_bytes());
        }
        ZIP_INT_32BIT => {
            let int = cursor.read_i32::<LittleEndian>()?;
            return Ok(int.to_string().into_bytes());
        }
        ZIP_INT_64BIT => {
            let int = cursor.read_i64::<LittleEndian>()?;
            return Ok(int.to_string().into_bytes());
        }
        _ => {
            let result = (flag - 0xF1) as isize;
            return Ok(result.to_string().into_bytes());
        }
    }
}
