-- SQL dump generated using DBML (dbml.dbdiagram.io)
-- Database: PostgreSQL
-- Generated at: 2024-12-03T17:35:25.615Z


CREATE TYPE "status" AS ENUM (
  'not_started',
  'in_progress',
  'completed',
  'wont_do'
);

CREATE TABLE "refactor_platform"."organizations" (
  "id" uuid UNIQUE PRIMARY KEY NOT NULL DEFAULT (gen_random_uuid()),
  "name" varchar NOT NULL,
  "logo" varchar,
  "created_at" timestamptz NOT NULL DEFAULT (now()),
  "updated_at" timestamptz NOT NULL DEFAULT (now())
);

CREATE TABLE "refactor_platform"."coaching_relationships" (
  "id" uuid UNIQUE PRIMARY KEY NOT NULL DEFAULT (gen_random_uuid()),
  "organization_id" uuid NOT NULL,
  "coach_id" uuid NOT NULL,
  "coachee_id" uuid NOT NULL,
  "created_at" timestamptz NOT NULL DEFAULT (now()),
  "updated_at" timestamptz NOT NULL DEFAULT (now())
);

CREATE TABLE "refactor_platform"."users" (
  "id" uuid UNIQUE PRIMARY KEY NOT NULL DEFAULT (gen_random_uuid()),
  "email" varchar UNIQUE NOT NULL,
  "first_name" varchar,
  "last_name" varchar,
  "display_name" varchar,
  "password" varchar NOT NULL,
  "github_username" varchar,
  "github_profile_url" varchar,
  "created_at" timestamptz NOT NULL DEFAULT (now()),
  "updated_at" timestamptz NOT NULL DEFAULT (now())
);

CREATE TABLE "refactor_platform"."coaching_sessions" (
  "id" uuid UNIQUE PRIMARY KEY NOT NULL DEFAULT (gen_random_uuid()),
  "coaching_relationship_id" uuid NOT NULL,
  "date" timestamp NOT NULL,
  "timezone" varchar NOT NULL,
  "created_at" timestamptz NOT NULL DEFAULT (now()),
  "updated_at" timestamptz NOT NULL DEFAULT (now())
);

CREATE TABLE "refactor_platform"."overarching_goals" (
  "id" uuid UNIQUE PRIMARY KEY NOT NULL DEFAULT (gen_random_uuid()),
  "user_id" uuid NOT NULL,
  "coaching_session_id" uuid,
  "title" varchar,
  "body" varchar,
  "status" status NOT NULL,
  "status_changed_at" timestamptz,
  "completed_at" timestamptz,
  "created_at" timestamptz NOT NULL DEFAULT (now()),
  "updated_at" timestamptz NOT NULL DEFAULT (now())
);

CREATE TABLE "refactor_platform"."notes" (
  "id" uuid UNIQUE PRIMARY KEY NOT NULL DEFAULT (gen_random_uuid()),
  "coaching_session_id" uuid NOT NULL,
  "body" varchar,
  "user_id" uuid NOT NULL,
  "created_at" timestamptz NOT NULL DEFAULT (now()),
  "updated_at" timestamptz NOT NULL DEFAULT (now())
);

CREATE TABLE "refactor_platform"."agreements" (
  "id" uuid UNIQUE PRIMARY KEY NOT NULL DEFAULT (gen_random_uuid()),
  "coaching_session_id" uuid NOT NULL,
  "body" varchar,
  "user_id" uuid NOT NULL,
  "created_at" timestamptz NOT NULL DEFAULT (now()),
  "updated_at" timestamptz NOT NULL DEFAULT (now())
);

CREATE TABLE "refactor_platform"."actions" (
  "id" uuid UNIQUE PRIMARY KEY NOT NULL DEFAULT (gen_random_uuid()),
  "coaching_session_id" uuid NOT NULL,
  "body" varchar,
  "user_id" uuid NOT NULL,
  "due_by" timestamptz,
  "status" status NOT NULL,
  "status_changed_at" timestamptz NOT NULL DEFAULT (now()),
  "created_at" timestamptz NOT NULL DEFAULT (now()),
  "updated_at" timestamptz NOT NULL DEFAULT (now())
);

COMMENT ON COLUMN "refactor_platform"."organizations"."name" IS 'The name of the organization that the coach <--> coachee belong to';

COMMENT ON COLUMN "refactor_platform"."organizations"."logo" IS 'A URI pointing to the organization''s logo icon file';

COMMENT ON COLUMN "refactor_platform"."organizations"."updated_at" IS 'The last date and time fields were changed';

COMMENT ON COLUMN "refactor_platform"."coaching_relationships"."organization_id" IS 'The organization associated with this coaching relationship';

COMMENT ON COLUMN "refactor_platform"."coaching_relationships"."coach_id" IS 'The coach associated with this coaching relationship';

COMMENT ON COLUMN "refactor_platform"."coaching_relationships"."coachee_id" IS 'The coachee associated with this coaching relationship';

COMMENT ON COLUMN "refactor_platform"."coaching_relationships"."updated_at" IS 'The last date and time fields were changed';

COMMENT ON COLUMN "refactor_platform"."users"."display_name" IS 'If a user wants to go by something other than first & last names';

COMMENT ON COLUMN "refactor_platform"."users"."updated_at" IS 'The last date and time fields were changed';

COMMENT ON COLUMN "refactor_platform"."coaching_sessions"."coaching_relationship_id" IS 'The coaching relationship (i.e. what coach & coachee under what organization) associated with this coaching session';

COMMENT ON COLUMN "refactor_platform"."coaching_sessions"."date" IS 'The date and time of a session';

COMMENT ON COLUMN "refactor_platform"."coaching_sessions"."timezone" IS 'The baseline timezone used for the `date` field';

COMMENT ON COLUMN "refactor_platform"."coaching_sessions"."updated_at" IS 'The last date and time fields were changed';

COMMENT ON COLUMN "refactor_platform"."overarching_goals"."user_id" IS 'User that created (owns) the overarching goal';

COMMENT ON COLUMN "refactor_platform"."overarching_goals"."coaching_session_id" IS 'The coaching session that an overarching goal is associated with';

COMMENT ON COLUMN "refactor_platform"."overarching_goals"."title" IS 'A short description of an overarching goal';

COMMENT ON COLUMN "refactor_platform"."overarching_goals"."body" IS 'Main text of the overarching goal supporting Markdown';

COMMENT ON COLUMN "refactor_platform"."overarching_goals"."completed_at" IS 'The date and time an overarching goal was completed';

COMMENT ON COLUMN "refactor_platform"."overarching_goals"."updated_at" IS 'The last date and time fields were changed';

COMMENT ON COLUMN "refactor_platform"."notes"."body" IS 'Main text of the note supporting Markdown';

COMMENT ON COLUMN "refactor_platform"."notes"."user_id" IS 'User that created (owns) the note';

COMMENT ON COLUMN "refactor_platform"."notes"."updated_at" IS 'The last date and time a note''s fields were changed';

COMMENT ON COLUMN "refactor_platform"."agreements"."body" IS 'Either a short or long description of an agreement reached between coach and coachee in a coaching session';

COMMENT ON COLUMN "refactor_platform"."agreements"."user_id" IS 'User that created (owns) the agreement';

COMMENT ON COLUMN "refactor_platform"."agreements"."updated_at" IS 'The last date and time an agreement''s fields were changed';

COMMENT ON COLUMN "refactor_platform"."actions"."body" IS 'Main text of the action supporting Markdown';

COMMENT ON COLUMN "refactor_platform"."actions"."user_id" IS 'User that created (owns) the action';

ALTER TABLE "refactor_platform"."coaching_relationships" ADD FOREIGN KEY ("organization_id") REFERENCES "refactor_platform"."organizations" ("id");

ALTER TABLE "refactor_platform"."coaching_relationships" ADD FOREIGN KEY ("coachee_id") REFERENCES "refactor_platform"."users" ("id");

ALTER TABLE "refactor_platform"."coaching_relationships" ADD FOREIGN KEY ("coach_id") REFERENCES "refactor_platform"."users" ("id");

ALTER TABLE "refactor_platform"."coaching_sessions" ADD FOREIGN KEY ("coaching_relationship_id") REFERENCES "refactor_platform"."coaching_relationships" ("id");

ALTER TABLE "refactor_platform"."overarching_goals" ADD FOREIGN KEY ("coaching_session_id") REFERENCES "refactor_platform"."coaching_sessions" ("id");

ALTER TABLE "refactor_platform"."notes" ADD FOREIGN KEY ("coaching_session_id") REFERENCES "refactor_platform"."coaching_sessions" ("id");

ALTER TABLE "refactor_platform"."notes" ADD FOREIGN KEY ("user_id") REFERENCES "refactor_platform"."users" ("id");

ALTER TABLE "refactor_platform"."agreements" ADD FOREIGN KEY ("coaching_session_id") REFERENCES "refactor_platform"."coaching_sessions" ("id");

ALTER TABLE "refactor_platform"."agreements" ADD FOREIGN KEY ("user_id") REFERENCES "refactor_platform"."users" ("id");

ALTER TABLE "refactor_platform"."actions" ADD FOREIGN KEY ("coaching_session_id") REFERENCES "refactor_platform"."coaching_sessions" ("id");
