//! 账号业务逻辑：格式校验、argon2id 哈希、token 生成、注册/登录/Resume/资料更新。
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use rand::rngs::OsRng;
use rand::RngCore;

use crate::db::{Db, DbError};

/// 头像上限（字节）：客户端缩 256×256 JPEG q85 一般 10–30KB
pub const AVATAR_MAX: usize = 64 * 1024;

/// 认证成功后的身份凭证束
#[derive(Debug)]
pub struct AuthResult {
    pub account_id: i64,
    pub nickname: String,
    pub has_avatar: bool,
    /// 供下次自动登录的会话 token（32 位 hex）
    pub auth_token: String,
}

pub fn validate_account(account: &str) -> Result<(), String> {
    let ok = (3..=20).contains(&account.len())
        && !account.starts_with('_')
        && account.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if ok {
        Ok(())
    } else {
        Err("账号格式不合法（3-20 位字母数字下划线，不能以 _ 开头）".into())
    }
}

pub fn validate_password(password: &str) -> Result<(), String> {
    let n = password.chars().count();
    if n < 6 {
        return Err("密码至少 6 位".into());
    }
    if n > 64 {
        return Err("密码过长（上限 64 字符）".into());
    }
    Ok(())
}

pub fn validate_nickname(nickname: &str) -> Result<(), String> {
    let n = nickname.chars().count();
    if (1..=24).contains(&n) {
        Ok(())
    } else {
        Err("昵称不合法（1-24 字符）".into())
    }
}

/// argon2id（PHC 字符串自带盐与参数，直接入库）
pub fn hash_password(password: &str) -> anyhow::Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("哈希失败: {e}"))?;
    Ok(hash.to_string())
}

pub fn verify_password(password: &str, phc: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(phc) else {
        return false;
    };
    Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok()
}

/// 16 字节真随机 → 32 位 hex
pub fn new_auth_token() -> String {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 注册：邀请码校验 → 格式校验 → 建号（昵称 = 账号名）→ 发 token
pub fn register(
    db: &Db,
    invite_cfg: Option<&str>,
    account: &str,
    password: &str,
    invite: &str,
) -> Result<AuthResult, String> {
    let Some(expected) = invite_cfg else {
        return Err("未开放注册".into());
    };
    if invite != expected {
        return Err("邀请码错误".into());
    }
    let account = account.trim().to_string();
    validate_account(&account)?;
    validate_password(password)?;
    let hash = hash_password(password).map_err(|e| format!("服务器错误：{e}"))?;
    let id = match db.create_account(&account, &hash, &account) {
        Ok(id) => id,
        Err(DbError::AccountExists) => return Err("账号已存在".into()),
        Err(DbError::Other(e)) => return Err(format!("服务器错误：{e}")),
    };
    let token = new_auth_token();
    db.insert_token(id, &token).map_err(|e| format!("服务器错误：{e}"))?;
    Ok(AuthResult { account_id: id, nickname: account, has_avatar: false, auth_token: token })
}

/// 登录：查账号 → argon2 校验 → 发新 token（失败统一文案，不泄露账号是否存在）
pub fn login(db: &Db, account: &str, password: &str) -> Result<AuthResult, String> {
    let account = account.trim();
    let Some(acc) = db.find_account(&account) else {
        return Err("账号或密码错误".into());
    };
    if !verify_password(password, &acc.password_hash) {
        return Err("账号或密码错误".into());
    }
    let token = new_auth_token();
    db.insert_token(acc.id, &token).map_err(|e| format!("服务器错误：{e}"))?;
    Ok(AuthResult {
        account_id: acc.id,
        nickname: acc.nickname,
        has_avatar: acc.has_avatar,
        auth_token: token,
    })
}

/// 自动登录：token 换身份（失效/超 24h 未用 → 客户端回登录页）
pub fn resume(db: &Db, auth_token: &str) -> Result<AuthResult, String> {
    let Some(acc) = db.find_account_by_token(auth_token) else {
        return Err("登录已过期".into());
    };
    // 滑动续期：本次使用成功即刷新 24h 有效期（活跃则可持续免登）
    db.touch_token(auth_token).map_err(|e| format!("服务器错误：{e}"))?;
    Ok(AuthResult {
        account_id: acc.id,
        nickname: acc.nickname,
        has_avatar: acc.has_avatar,
        auth_token: auth_token.to_string(),
    })
}

/// 资料更新：昵称必填；头像 Some 才更新（None = 保持原样）
pub fn apply_profile(
    db: &Db,
    account_id: i64,
    nickname: &str,
    avatar: Option<&[u8]>,
) -> Result<(), String> {
    validate_nickname(nickname)?;
    if let Some(data) = avatar {
        if data.is_empty() {
            return Err("头像数据为空".into());
        }
        if data.len() > AVATAR_MAX {
            return Err("头像过大（上限 64KB）".into());
        }
    }
    db.update_nickname(account_id, nickname).map_err(|e| format!("服务器错误：{e}"))?;
    if let Some(data) = avatar {
        db.update_avatar(account_id, data).map_err(|e| format!("服务器错误：{e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem_db() -> Db {
        Db::open_in_memory()
    }

    #[test]
    fn register_then_login_and_resume() {
        let db = mem_db();
        let r = register(&db, Some("code-1"), "Alice", "pw123456", "code-1").unwrap();
        // 账号保留原始大小写、昵称默认 = 账号名
        assert_eq!(r.nickname, "Alice");
        assert_eq!(r.auth_token.len(), 32);
        assert!(!r.has_avatar);

        let l = login(&db, "Alice", "pw123456").unwrap();
        assert_eq!(l.account_id, r.account_id);
        assert_ne!(l.auth_token, r.auth_token, "每次登录发新 token");
        // 大小写敏感：小写形式是另一个账号，查不到
        assert_eq!(login(&db, "alice", "pw123456").unwrap_err(), "账号或密码错误");

        let s = resume(&db, &l.auth_token).unwrap();
        assert_eq!(s.account_id, r.account_id);
        assert_eq!(s.nickname, "Alice");
        assert_eq!(s.auth_token, l.auth_token);
    }

    #[test]
    fn register_error_cases() {
        let db = mem_db();
        assert_eq!(register(&db, None, "alice", "pw123456", "x").unwrap_err(), "未开放注册");
        assert_eq!(
            register(&db, Some("code-1"), "alice", "pw123456", "wrong").unwrap_err(),
            "邀请码错误"
        );
        assert_eq!(
            register(&db, Some("code-1"), "ab", "pw123456", "code-1").unwrap_err(),
            "账号格式不合法（3-20 位字母数字下划线，不能以 _ 开头）"
        );
        assert_eq!(
            register(&db, Some("code-1"), "alice中文", "pw123456", "code-1").unwrap_err(),
            "账号格式不合法（3-20 位字母数字下划线，不能以 _ 开头）"
        );
        assert_eq!(
            register(&db, Some("code-1"), "_alice", "pw123456", "code-1").unwrap_err(),
            "账号格式不合法（3-20 位字母数字下划线，不能以 _ 开头）"
        );
        assert_eq!(
            register(&db, Some("code-1"), "alice", "12345", "code-1").unwrap_err(),
            "密码至少 6 位"
        );
        // 大小写敏感：Alice 与 alice 是两个独立账号，可各自注册
        register(&db, Some("code-1"), "Alice", "pw123456", "code-1").unwrap();
        register(&db, Some("code-1"), "alice", "pw123456", "code-1").unwrap();
        // 完全一致（含大小写）才报已存在
        assert_eq!(
            register(&db, Some("code-1"), "Alice", "pw123456", "code-1").unwrap_err(),
            "账号已存在"
        );
    }

    #[test]
    fn login_unified_error() {
        let db = mem_db();
        register(&db, Some("code-1"), "alice", "pw123456", "code-1").unwrap();
        assert_eq!(login(&db, "alice", "wrong-pw").unwrap_err(), "账号或密码错误");
        assert_eq!(login(&db, "nobody", "pw123456").unwrap_err(), "账号或密码错误");
        assert_eq!(resume(&db, "不存在的token").unwrap_err(), "登录已过期");
    }

    #[test]
    fn apply_profile_validates_and_updates() {
        let db = mem_db();
        let r = register(&db, Some("code-1"), "alice", "pw123456", "code-1").unwrap();
        let img = vec![9u8; 1024];

        apply_profile(&db, r.account_id, "阿信", Some(&img)).unwrap();
        let acc = db.find_account("alice").unwrap();
        assert_eq!(acc.nickname, "阿信");
        assert!(acc.has_avatar);
        assert_eq!(db.get_avatar(r.account_id).unwrap(), img);

        // 只改昵称：avatar = None 不动头像
        apply_profile(&db, r.account_id, "信哥", None).unwrap();
        assert_eq!(db.find_account("alice").unwrap().nickname, "信哥");
        assert_eq!(db.get_avatar(r.account_id).unwrap(), img);

        assert_eq!(apply_profile(&db, r.account_id, "", None).unwrap_err(), "昵称不合法（1-24 字符）");
        assert_eq!(
            apply_profile(&db, r.account_id, &"名".repeat(25), None).unwrap_err(),
            "昵称不合法（1-24 字符）"
        );
        let big = vec![0u8; AVATAR_MAX + 1];
        assert_eq!(
            apply_profile(&db, r.account_id, "阿信", Some(&big)).unwrap_err(),
            "头像过大（上限 64KB）"
        );
    }

    #[test]
    fn password_hash_roundtrip() {
        let phc = hash_password("pw123456").unwrap();
        assert!(phc.starts_with("$argon2"));
        assert!(verify_password("pw123456", &phc));
        assert!(!verify_password("wrong", &phc));
        assert!(!verify_password("pw123456", "不是PHC字符串"));
    }
}
