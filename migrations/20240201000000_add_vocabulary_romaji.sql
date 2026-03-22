-- Add romaji column to vocabulary table
ALTER TABLE vocabulary ADD COLUMN romaji TEXT NOT NULL DEFAULT '';
