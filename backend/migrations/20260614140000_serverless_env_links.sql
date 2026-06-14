-- Selective links from serverless functions to the project env pool (parity with
-- app_env_links for app instances). A linked pool var is injected at deploy as a
-- live reference (resolved fresh from project_env_variables each deploy).
CREATE TABLE serverless_env_links (
    function_id    UUID NOT NULL REFERENCES serverless_functions(id) ON DELETE CASCADE,
    project_env_id UUID NOT NULL REFERENCES project_env_variables(id) ON DELETE CASCADE,
    PRIMARY KEY (function_id, project_env_id)
);
