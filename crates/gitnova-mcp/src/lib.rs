pub mod resources;
pub mod rmcp_server;
pub mod schema;
pub mod server;
pub mod tools;

pub use server::serve_stdio;

#[cfg(test)]
mod tests {
    #[test]
    fn rmcp_adapter_is_a_real_server_handler() {
        let temp = tempfile::TempDir::new().unwrap();
        let server = crate::rmcp_server::GitnovaRmcpServer::new(temp.path().to_path_buf());

        fn assert_handler<T: rmcp::ServerHandler>(_handler: &T) {}
        assert_handler(&server);

        let tools = server.tool_names();
        assert!(tools.contains(&"index_project".to_string()));
        assert!(tools.contains(&"watch_status".to_string()));
    }
}
