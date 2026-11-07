-- The last history row per player that named an episode or movie, as JSON,
-- so a row without a title can continue it after other apps played.
ALTER TABLE playback_cursor ADD COLUMN last TEXT;
