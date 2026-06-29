# Auto TURSO table

a drop-in *dependency* for easier work with SQL tables.

> Something similar to JPA for table definitions 

1. It can automatically synchronize structs with SQL tables
2. Do some basic DML and DQL stuff
3. *that's it*, it just helps with the boilerplate stuff

> [!NOTE]
> This system is written for the [turso](https://github.com/tursodatabase/turso) database. It might work with other SQLITE3 crates, but you would have to add your own abstraction implementation just like [[crate::conn::turso]](./auto_table/src/conn/turso.rs).

Just create an `AutoTable`
```rust
use auto_table::*;

#[derive(Serialize, Clone, AutoTable)]
#[auto_table(index_by="sent_at DESC")]
pub struct Message {
    #[auto_table(primary_key)]
    pub id: u64,
    pub author_id: u64,
    #[auto_table(data_type="INTEGER", with = "ChannelCode::to_sql,ChannelCode::from_sql")]
    pub channel_code: ChannelCode,
    #[auto_table(with = "id_utils::option_id_to_sql,id_utils::option_id_from_sql")]
    pub nonce_id: Option<u64>,
    pub content: String,
    #[auto_table(data_type="BIGINT", sort_desc)]
    pub sent_at: Timestamp, // AtSqlCodec implemented
}
```

And do stuff with it
```rust
use auto_table::sync_table;
use auto_table::queries::{insert_into, get_in_range};

impl MessageManager {
    async fn start(mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let db = turso::Builder::new_local("my_db.db").build().await?;
        let conn = db.connect()?;
        
        // Automatically creates or updates table based on AutoTable implemented struct
        sync_table::<Message>(&conn).await?;
        
        self.db_conn = conn;
    }
    
    async fn foo(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Inserts (or replaces) a new row in the "messages" table
        insert_into::<Message>(&*self.db_conn, &msg).await?;
    }

    pub async fn get_messages(&self, page: u16) -> Result<Vec<Message>, Box<dyn std::error::Error + Send + Sync>> {
        // Gets all rows from database within specified range (limit, offset)
        get_in_range::<Message>(db_conn, 100, 100 * (page as u64)).await
    }
}
```

**Documentation WHERE?**

That's the neat part. I made this for myself and I know (*I hope*) how it works. So good luck using it :D

**If you dare to use it**

Instead of trying to find this on crates, just clone this repo into yours (via git modules or a zip file, idc). Thisway it's going to be easier for both you (to edit and fix my problems) and me (to not having to add stuff and fix bugs).

```toml
[workspace]
members = ["your_project", "auto-turso-table/auto_table", "auto-turso-table/auto_table_derive"]
```

```toml
[dependencies]
auto_table = { path = "../auto-turso-table/auto_table" }
```

**AI disclosure**

A part of this *project* was AI-assisted (by Claude Sonnet 4.6 I think). To be honest, he did a great work, but like 10% of it...
At least I've learned how the rust derive system works ¯\\(ツ)/¯.


### How to do custom data types

```rust
use auto_table::*;

#[derive(AutoTable)]
pub struct User {
  pub id: u64,
  pub name: String,
  #[auto_table(data_type="INTEGER")] // automatically serializes based on `AtSqlCodec`
  // OR: #[auto_table(data_type="INTEGER", with="UserFeatures::to_sql,UserFeatures::from_sql")]
  pub features: UserFeatures,
  pub signed_status: Option<u8>, // supports Options
}

// Using bitflags crate here...
bitflags! {
    pub struct UserFeatures: u16 {
        const NONE        = 0x00;
        const FEATURE_ONE = 0x01;
        const FEATURE_TWO = 0x02;
    }
}

impl AtSqlCodec for UserFeatures {
  fn to_sql(&self) -> impl AtIntoValue {
    (self.bits()).to_string()
  }

  fn from_sql(value: &Value) -> Result<Self, SqlError> {
    let mut v: Option<u16> = None;
    if value.is_integer() {
      v = value.as_integer().map(|e| e.clone() as u16)
    } else if let Some(e) = value.as_text() {
      v = Some(e.trim().parse().map_err(|e| SqlError::InvalidData(format!("{}",e)))?);
    }
    match v {
      None => Err(SqlError::InvalidData(String::from("Missing value"))),
      Some(val) => Ok(UserFeatures::from_bits_truncate(val))
    }
  }
}
```

