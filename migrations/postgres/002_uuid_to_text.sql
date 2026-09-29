-- Convert legacy UUID columns to TEXT for app-generated UUID strings.
-- Safe to run on fresh TEXT installs (no-op-ish) and on old UUID installs.

DO $$ BEGIN
  BEGIN
    ALTER TABLE users ALTER COLUMN id DROP DEFAULT;
  EXCEPTION WHEN OTHERS THEN NULL;
  END;
  BEGIN
    ALTER TABLE posts ALTER COLUMN id DROP DEFAULT;
  EXCEPTION WHEN OTHERS THEN NULL;
  END;
  BEGIN
    ALTER TABLE tags ALTER COLUMN id DROP DEFAULT;
  EXCEPTION WHEN OTHERS THEN NULL;
  END;
  BEGIN
    ALTER TABLE comments ALTER COLUMN id DROP DEFAULT;
  EXCEPTION WHEN OTHERS THEN NULL;
  END;
END $$;

-- Only convert when the column is actually UUID (fresh TEXT installs skip the heavy rewrite).
DO $$ BEGIN
  IF EXISTS (
    SELECT 1 FROM information_schema.columns
    WHERE table_name = 'users' AND column_name = 'id' AND udt_name = 'uuid'
  ) THEN
    ALTER TABLE users ALTER COLUMN id TYPE TEXT USING id::text;
  END IF;
  IF EXISTS (
    SELECT 1 FROM information_schema.columns
    WHERE table_name = 'posts' AND column_name = 'id' AND udt_name = 'uuid'
  ) THEN
    ALTER TABLE posts ALTER COLUMN id TYPE TEXT USING id::text;
  END IF;
  IF EXISTS (
    SELECT 1 FROM information_schema.columns
    WHERE table_name = 'posts' AND column_name = 'author_id' AND udt_name = 'uuid'
  ) THEN
    ALTER TABLE posts ALTER COLUMN author_id TYPE TEXT USING author_id::text;
  END IF;
  IF EXISTS (
    SELECT 1 FROM information_schema.columns
    WHERE table_name = 'tags' AND column_name = 'id' AND udt_name = 'uuid'
  ) THEN
    ALTER TABLE tags ALTER COLUMN id TYPE TEXT USING id::text;
  END IF;
  IF EXISTS (
    SELECT 1 FROM information_schema.columns
    WHERE table_name = 'post_tags' AND column_name = 'post_id' AND udt_name = 'uuid'
  ) THEN
    ALTER TABLE post_tags ALTER COLUMN post_id TYPE TEXT USING post_id::text;
  END IF;
  IF EXISTS (
    SELECT 1 FROM information_schema.columns
    WHERE table_name = 'post_tags' AND column_name = 'tag_id' AND udt_name = 'uuid'
  ) THEN
    ALTER TABLE post_tags ALTER COLUMN tag_id TYPE TEXT USING tag_id::text;
  END IF;
  IF EXISTS (
    SELECT 1 FROM information_schema.columns
    WHERE table_name = 'comments' AND column_name = 'id' AND udt_name = 'uuid'
  ) THEN
    ALTER TABLE comments ALTER COLUMN id TYPE TEXT USING id::text;
  END IF;
  IF EXISTS (
    SELECT 1 FROM information_schema.columns
    WHERE table_name = 'comments' AND column_name = 'post_id' AND udt_name = 'uuid'
  ) THEN
    ALTER TABLE comments ALTER COLUMN post_id TYPE TEXT USING post_id::text;
  END IF;
  IF EXISTS (
    SELECT 1 FROM information_schema.columns
    WHERE table_name = 'comments' AND column_name = 'author_id' AND udt_name = 'uuid'
  ) THEN
    ALTER TABLE comments ALTER COLUMN author_id TYPE TEXT USING author_id::text;
  END IF;
  IF EXISTS (
    SELECT 1 FROM information_schema.columns
    WHERE table_name = 'comments' AND column_name = 'parent_id' AND udt_name = 'uuid'
  ) THEN
    ALTER TABLE comments ALTER COLUMN parent_id TYPE TEXT USING parent_id::text;
  END IF;
END $$;
