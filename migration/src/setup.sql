-- TODO: Is it possible to make the DB name, DB user, schema name, etc all environment variables from the container?

CREATE SCHEMA refactor_platform AUTHORIZATION refactor;

GRANT ALL PRIVILEGES ON DATABASE refactor TO refactor;
GRANT ALL ON SCHEMA refactor_platform TO refactor;

ALTER DEFAULT PRIVILEGES IN SCHEMA refactor_platform GRANT ALL ON TABLES TO refactor;
ALTER DEFAULT PRIVILEGES IN SCHEMA refactor_platform GRANT ALL ON SEQUENCES TO refactor;
ALTER DEFAULT PRIVILEGES IN SCHEMA refactor_platform GRANT ALL ON FUNCTIONS TO refactor;