CREATE TABLE IF NOT EXISTS vocabulary (
    id INTEGER PRIMARY KEY,
    word TEXT NOT NULL UNIQUE,
    reading TEXT NOT NULL,
    meaning TEXT NOT NULL,
    level TEXT DEFAULT 'N5',
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS grammar (
    id INTEGER PRIMARY KEY,
    pattern TEXT NOT NULL UNIQUE,
    meaning TEXT NOT NULL,
    level TEXT DEFAULT 'N5',
    example TEXT,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS reviews (
    id INTEGER PRIMARY KEY,
    item_id INTEGER NOT NULL,
    item_type TEXT NOT NULL,
    reviewed_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    quality INTEGER NOT NULL,
    interval_days REAL NOT NULL,
    ease REAL NOT NULL,
    reps INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS user_profile (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

INSERT OR IGNORE INTO user_profile (key, value) VALUES ('current_level', 'N5');
INSERT OR IGNORE INTO user_profile (key, value) VALUES ('native_language', 'zh');
INSERT OR IGNORE INTO user_profile (key, value) VALUES ('target_language', 'ja');
