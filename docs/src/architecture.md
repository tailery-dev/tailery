# How it Works

Tailery is built entirely in **Rust** for maximum performance and safety. It acts as an intermediary layer between your MCP servers and your Coding Assistants.

## Architecture Overview

At a high level, Tailery works through three main concepts:

### 1. State and Profiles (`src/state.rs`)
Tailery maintains an `AppState` containing all your servers, skills, and settings. To allow for flexible workflows, Tailery introduces **Profiles**. A Profile determines which MCP servers and clients are enabled at any given time.

### 2. Client Adapters (`src/adapters/`)
Different IDEs store their MCP configurations in different formats and locations. 
- **Cursor** uses a JSON file at `~/.cursor/mcp.json` (or similar depending on OS).
- **Claude Code** and **Zed** have their own configuration structures.

Tailery abstracts this away using **Adapters**. When you run `tailery sync`, it iterates over all supported adapters, reads the active profile, translates the unified server configurations into the IDE's specific format, and safely writes to the IDE's config file.

### 3. Shim Interceptor (`src/shim.rs`)
To provide fine-grained control and security over what your AI assistants can do, Tailery acts as a **Shim**. Instead of the IDE communicating directly with the MCP server, Tailery can wrap the execution.

This allows Tailery to enforce **Tool Filters**:
- `allow`: Only permit specific tools (e.g., `read_file`).
- `deny`: Block dangerous tools (e.g., `delete_file`).
- `auto_approve`: Skip manual confirmations for safe tools.

### 4. Docker Integration (`src/docker.rs`)
For local servers that require isolation, Tailery natively integrates with the Docker socket. It can automatically pull images, mount necessary workspaces, set resource limits (CPU/Memory), and start MCP servers inside ephemeral containers, passing the I/O streams securely back to the IDE.
