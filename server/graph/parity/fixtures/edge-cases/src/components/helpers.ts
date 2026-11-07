export const cast = <T,>(x: unknown) => <T>x;
export const sql = `SELECT u.id, u.email, o.total FROM users u LEFT JOIN orders o ON o.user_id = u.id WHERE u.status = ${"x"} AND o.total > 10`;
export const upd = "UPDATE users SET email = ?, name = ? WHERE id = ?";
export const ins = `INSERT INTO orders (user_id, total) VALUES (?, ?) ON DUPLICATE KEY UPDATE total = VALUES(total)`;
export const ddl = `CREATE TABLE IF NOT EXISTS audit (id INT PRIMARY KEY, note TEXT, `key` VARCHAR(20))`;
export const re = /[`"']+/g; const d = 4 / 2 / 1;
