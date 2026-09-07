# Notes

credential renewal should surface auth_refresh
sanitize user input should surface validate_input
durable session write should surface persist_session
throttle inbound clients should surface rate_limit_client
debounce noisy updates should surface coalesce_watch_events
retry after transient failure should surface backoff_attempt
