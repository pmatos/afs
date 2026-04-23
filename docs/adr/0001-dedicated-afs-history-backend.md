# Use a Dedicated AFS History Backend in Agent Home

AFS stores **Directory History** in a dedicated **History Backend** inside each `.afs/` **Agent Home**, initially implementable with git but separate from any user or project repository. We chose this so undo works for both git and non-git folders, avoids polluting project history with agent activity, and keeps reversible state with the **Directory Agent** that owns the **Managed Subtree**.
