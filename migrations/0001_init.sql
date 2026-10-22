CREATE TABLE quotes (
    id         INTEGER PRIMARY KEY,
    text       TEXT NOT NULL,
    author     TEXT NOT NULL,
    work       TEXT,
    date       TEXT,
    language   TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE artworks (
    id         INTEGER PRIMARY KEY,
    url        TEXT NOT NULL,
    filename   TEXT NOT NULL,
    thumbnail  TEXT,
    artist     TEXT NOT NULL,
    work       TEXT NOT NULL,
    date       TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE definitions (
    id          INTEGER PRIMARY KEY,
    language    TEXT NOT NULL CHECK (language IN ('english', 'french')),
    word        TEXT NOT NULL,
    translation TEXT NOT NULL,
    definition  TEXT NOT NULL,
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE scriptures (
    id          INTEGER PRIMARY KEY,
    book        TEXT NOT NULL,
    chapter     INTEGER NOT NULL,
    verse_start INTEGER NOT NULL,
    verse_end   INTEGER,
    text        TEXT NOT NULL,
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Every pick ever made, rerolls included. The current pick for a kind is the
-- latest row for today whose item still exists. No foreign key: item_id points
-- into a different table depending on kind.
CREATE TABLE daily_picks (
    id        INTEGER PRIMARY KEY,
    day       TEXT NOT NULL,  -- YYYY-MM-DD, Europe/Paris, rolling over at 05:00
    kind      TEXT NOT NULL,  -- quote | artwork | word_en | word_fr | scripture
    item_id   INTEGER NOT NULL,
    picked_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX daily_picks_kind_day ON daily_picks (kind, day);
