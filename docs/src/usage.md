# Example Usage

Tailery provides both a rich Terminal User Interface (TUI) and a powerful CLI.

## Starting the TUI

To launch the interactive TUI, simply run:

```bash
tailery
```
or 
```bash
tailery tui
```

From the TUI, you can view your configured servers, monitor Docker container status, and toggle skills dynamically.

## Using the CLI

If you prefer command-line automation, Tailery has you covered.

### Listing Configurations

To see all profiles, servers, and connected IDEs:

```bash
tailery list
```

### Managing Profiles

Profiles allow you to group servers together. For example, you might want a `work` profile and a `personal` profile.

```bash
# Create a new profile and switch to it
tailery profile create --name work --switch

# Enable a server for the active profile
tailery profile enable-server --server github-context

# Switch back to default
tailery profile switch --name default
```

### Synchronizing with Coding Assistants

Once you have your profile configured, you can push the configuration to all your installed coding assistants (Cursor, Zed, etc.):

```bash
tailery sync
```

You can also target a specific client:

```bash
tailery sync --client cursor
```

### Backing Up and Restoring

Tailery automatically manages backups of your IDE configurations before overwriting them, ensuring you never lose your old settings.

```bash
# Create a manual backup
tailery backup create

# List all stored backups
tailery backup list

# Restore the latest backup for a specific client
tailery backup restore --client cursor
```
