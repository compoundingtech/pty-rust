/**
 * A test must not inherit the session the test runner is itself inside, or
 * the nesting guard turns every spawn into a direct exec.
 */
for (const key of ["PTY_SESSION", "PTY_SESSION_GENERATION", "PTY_SESSION_DIR", "PTY_ROOT", "PTY_REAP_ON_EXIT"]) {
  delete process.env[key];
}
