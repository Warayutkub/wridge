-- ตารางผู้ใช้
CREATE TABLE users (
  id INT AUTO_INCREMENT PRIMARY KEY,
  email VARCHAR(255) NOT NULL UNIQUE, -- อีเมล
  status ENUM('a','b') DEFAULT 'a'
);
CREATE TABLE orders (id INT PRIMARY KEY, user_id INT, total DECIMAL(10,2), FOREIGN KEY (user_id) REFERENCES users(id));
CREATE INDEX orders_user ON orders (user_id DESC);
FROBNICATE xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx 🎉🎉 tail;
ALTER TABLE users ADD COLUMN phone VARCHAR(20), RENAME COLUMN status TO state;
CREATE TABLE broken (id INT, name TEXT
