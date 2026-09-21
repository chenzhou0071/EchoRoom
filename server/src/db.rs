//! SQLite 持久层：账号、密码哈希、token（含 8 条修剪）；头像图片存文件系统
//! （<数据目录>/avatars/<account_id>.jpg），库内只留 has_avatar 标记。
//! 单连接 + Mutex 串行化（6 人规模足够）；rusqlite bundled 把 SQLite 编进二进制。
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{params, Connection};

/// 每账号最多保留的 auth_token 条数（超出删最旧，支持换设备）
pub const TOKEN_LIMIT: usize = 8;

/// 账号记录（has_avatar 为库内标记列：0 无头像 / 1 有头像）
#[derive(Debug, Clone, PartialEq)]
pub struct Account {
    pub id: i64,
    pub account: String,
    pub password_hash: String,
    pub nickname: String,
    pub has_avatar: bool,
}

#[derive(Debug)]
pub enum DbError {
    /// 账号 UNIQUE 约束冲突（并发注册兜底）
    AccountExists,
    Other(String),
}

pub struct Db {
    conn: Mutex<Connection>,
    /// 头像文件目录（`<db 文件所在目录>/avatars/<account_id>.jpg`）
    avatar_dir: PathBuf,
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn row_to_account(row: &rusqlite::Row<'_>) -> rusqlite::Result<Account> {
    Ok(Account {
        id: row.get(0)?,
        account: row.get(1)?,
        password_hash: row.get(2)?,
        nickname: row.get(3)?,
        has_avatar: row.get(4)?,
    })
}

impl Db {
    /// 打开（或创建）数据库文件并确保表结构；父目录与头像目录自动创建
    pub fn open(path: &Path) -> anyhow::Result<Db> {
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(dir)?;
        Db::with_conn(Connection::open(path)?, dir.join("avatars"))
    }

    /// 组装 Db：建头像目录 + 初始化表结构
    fn with_conn(conn: Connection, avatar_dir: PathBuf) -> anyhow::Result<Db> {
        std::fs::create_dir_all(&avatar_dir)?;
        let db = Db { conn: Mutex::new(conn), avatar_dir };
        db.init_schema()?;
        Ok(db)
    }

    /// 内存库（测试用；auth.rs 的测试也复用）；头像落盘临时目录（pid + 纳秒命名，互不干扰）
    #[cfg(test)]
    pub fn open_in_memory() -> Db {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let name = format!("echoroom-test-avatars-{}-{nanos}", std::process::id());
        Db::with_conn(Connection::open_in_memory().unwrap(), std::env::temp_dir().join(name))
            .unwrap()
    }

    fn init_schema(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS accounts (
               id            INTEGER PRIMARY KEY AUTOINCREMENT,
               account       TEXT NOT NULL UNIQUE,
               password_hash TEXT NOT NULL,
               nickname      TEXT NOT NULL,
               has_avatar    INTEGER NOT NULL DEFAULT 0,
               created_at    INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS tokens (
               token      TEXT PRIMARY KEY,
               account_id INTEGER NOT NULL,
               created_at INTEGER NOT NULL
             );",
        )?;
        Ok(())
    }

    /// 建账号；UNIQUE 冲突 → `AccountExists`
    pub fn create_account(
        &self,
        account: &str,
        password_hash: &str,
        nickname: &str,
    ) -> Result<i64, DbError> {
        let conn = self.conn.lock().unwrap();
        let r = conn.execute(
            "INSERT INTO accounts (account, password_hash, nickname, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![account, password_hash, nickname, now_secs()],
        );
        match r {
            Ok(_) => Ok(conn.last_insert_rowid()),
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Err(DbError::AccountExists)
            }
            Err(e) => Err(DbError::Other(e.to_string())),
        }
    }

    pub fn find_account(&self, account: &str) -> Option<Account> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, account, password_hash, nickname, has_avatar
             FROM accounts WHERE account = ?1",
            params![account],
            row_to_account,
        )
        .ok()
    }

    /// 写入 auth_token 并修剪：每账号只保留最新 TOKEN_LIMIT 条（同秒以 rowid 兜底）
    pub fn insert_token(&self, account_id: i64, token: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO tokens (token, account_id, created_at) VALUES (?1, ?2, ?3)",
            params![token, account_id, now_secs()],
        )?;
        conn.execute(
            "DELETE FROM tokens WHERE account_id = ?1 AND token NOT IN (
               SELECT token FROM tokens WHERE account_id = ?1
               ORDER BY created_at DESC, rowid DESC LIMIT ?2
             )",
            params![account_id, TOKEN_LIMIT as i64],
        )?;
        Ok(())
    }

    pub fn find_account_by_token(&self, token: &str) -> Option<Account> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT a.id, a.account, a.password_hash, a.nickname, a.has_avatar
             FROM tokens t JOIN accounts a ON a.id = t.account_id
             WHERE t.token = ?1",
            params![token],
            row_to_account,
        )
        .ok()
    }

    pub fn update_nickname(&self, account_id: i64, nickname: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE accounts SET nickname = ?1 WHERE id = ?2",
            params![nickname, account_id],
        )?;
        Ok(())
    }

    /// 写头像文件后置库内标记；顺序=先文件后库（最坏留孤儿文件，无害；下次上传覆盖）
    pub fn update_avatar(&self, account_id: i64, avatar: &[u8]) -> anyhow::Result<()> {
        std::fs::write(self.avatar_dir.join(format!("{account_id}.jpg")), avatar)?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE accounts SET has_avatar = 1 WHERE id = ?1",
            params![account_id],
        )?;
        Ok(())
    }

    /// 读头像文件（不存在 → None）
    pub fn get_avatar(&self, account_id: i64) -> Option<Vec<u8>> {
        std::fs::read(self.avatar_dir.join(format!("{account_id}.jpg"))).ok()
    }

    /// 是否有头像（读标记列，不碰文件）
    pub fn has_avatar(&self, account_id: i64) -> bool {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT has_avatar FROM accounts WHERE id = ?1",
            params![account_id],
            |row| row.get::<_, bool>(0),
        )
        .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_find_account() {
        let db = Db::open_in_memory();
        let id = db.create_account("alice", "hash1", "alice").unwrap();
        let acc = db.find_account("alice").unwrap();
        assert_eq!(acc.id, id);
        assert_eq!(acc.nickname, "alice");
        assert_eq!(acc.password_hash, "hash1");
        assert!(!acc.has_avatar);
        assert!(db.find_account("bob").is_none());
    }

    #[test]
    fn duplicate_account_is_constraint_error() {
        let db = Db::open_in_memory();
        db.create_account("alice", "h1", "alice").unwrap();
        assert!(matches!(db.create_account("alice", "h2", "阿信"), Err(DbError::AccountExists)));
    }

    #[test]
    fn token_insert_find_and_trim() {
        let db = Db::open_in_memory();
        let id = db.create_account("alice", "h1", "alice").unwrap();
        db.insert_token(id, "tok-0").unwrap();
        for i in 1..10 {
            db.insert_token(id, &format!("tok-{i}")).unwrap();
        }
        // 上限 8 条：最旧的两条被修剪，最新的仍在
        assert!(db.find_account_by_token("tok-0").is_none(), "最旧 token 应被修剪");
        assert!(db.find_account_by_token("tok-1").is_none());
        assert_eq!(db.find_account_by_token("tok-2").unwrap().id, id);
        assert_eq!(db.find_account_by_token("tok-9").unwrap().account, "alice");
        assert!(db.find_account_by_token("不存在").is_none());
    }

    #[test]
    fn update_nickname_and_avatar() {
        let db = Db::open_in_memory();
        let id = db.create_account("alice", "h1", "alice").unwrap();
        assert!(!db.has_avatar(id));
        db.update_nickname(id, "阿信").unwrap();
        assert_eq!(db.find_account("alice").unwrap().nickname, "阿信");
        let img = vec![1u8, 2, 3, 4, 5];
        db.update_avatar(id, &img).unwrap();
        assert!(db.has_avatar(id));
        assert!(db.avatar_dir.join(format!("{id}.jpg")).exists(), "头像应落盘为文件");
        assert_eq!(db.get_avatar(id).unwrap(), img);
        assert!(db.find_account("alice").unwrap().has_avatar);
    }

    #[test]
    fn get_avatar_none_when_missing() {
        let db = Db::open_in_memory();
        let id = db.create_account("alice", "h1", "alice").unwrap();
        assert_eq!(db.get_avatar(id), None);
        assert_eq!(db.get_avatar(999), None);
        assert!(!db.has_avatar(999));
    }
}
