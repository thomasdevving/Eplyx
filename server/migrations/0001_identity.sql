-- Mutable identity and authorization only. No project/run/evidence registry.
CREATE TABLE users (
    id TEXT PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE sessions (
    token_sha256 TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE workspaces (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    created_by TEXT NOT NULL REFERENCES users(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE workspace_members (
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('owner', 'member')),
    added_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, user_id)
);

-- Projects are visible to their workspace's members only. There is no public
-- visibility; the single optional demo project is chosen by server config.
-- Authorization mapping only. The filesystem project record owns identity,
-- name, program/asset scope and all analytical history.
CREATE TABLE project_workspaces (
    project_id TEXT PRIMARY KEY CHECK (project_id ~ '^proj_[0-9A-HJKMNP-TV-Z]{26}$'),
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    linked_by TEXT NOT NULL REFERENCES users(id),
    linked_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Only SHA-256 digests of tokens are stored. `user` tokens come from the CLI
-- device flow; project tokens are scoped to one project and can submit checks, sync and read its results.
CREATE TABLE api_tokens (
    id TEXT PRIMARY KEY,
    token_sha256 TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL CHECK (kind IN ('user', 'project')),
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    project_id TEXT REFERENCES project_workspaces(project_id) ON DELETE CASCADE,
    label TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ,
    last_used_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    CHECK ((kind = 'project') = (project_id IS NOT NULL))
);

CREATE TABLE device_codes (
    device_sha256 TEXT PRIMARY KEY,
    user_code TEXT NOT NULL UNIQUE,
    client TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL,
    last_poll_at TIMESTAMPTZ,
    approved_by TEXT REFERENCES users(id) ON DELETE CASCADE,
    approved_at TIMESTAMPTZ,
    denied BOOLEAN NOT NULL DEFAULT false,
    consumed_at TIMESTAMPTZ
);

-- Which stable local project IDs feed a cloud project (developer machines
-- and CI checkouts each have their own).
CREATE TABLE project_links (
    project_id TEXT NOT NULL REFERENCES project_workspaces(project_id) ON DELETE CASCADE,
    local_project_id TEXT NOT NULL,
    linked_by TEXT NOT NULL,
    linked_via TEXT NOT NULL CHECK (linked_via IN ('cli', 'ci')),
    linked_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, local_project_id)
);

CREATE INDEX workspace_members_user ON workspace_members(user_id);
CREATE INDEX project_workspaces_workspace ON project_workspaces(workspace_id);
