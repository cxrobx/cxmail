import { useState, useCallback, useRef, useEffect } from "react";
import { cn } from "@/lib/utils";
import { useMailStore } from "@/stores/mailStore";
import { api } from "@/lib/tauri";
import { Inbox, Send, FileText, Trash2, AlertTriangle, Archive, Star, Folder, ChevronRight, Plus, Pencil } from "lucide-react";
import type { Folder as FolderType } from "@/types/email";

const FOLDER_ICONS: Record<string, typeof Inbox> = {
  inbox: Inbox,
  sent: Send,
  drafts: FileText,
  trash: Trash2,
  spam: AlertTriangle,
  archive: Archive,
  starred: Star,
};

const SORT_ORDER = ["inbox", "drafts", "sent", "starred", "archive", "spam", "trash"];

interface FolderNode {
  folder: FolderType | null; // null for virtual parent nodes
  label: string;
  children: FolderNode[];
}

function getFolderDisplayName(folder: FolderType): string {
  if (folder.display_name) return folder.display_name;
  const name = folder.name;
  if (name === "INBOX") return "Inbox";
  if (name.startsWith("[Gmail]/")) return name.replace("[Gmail]/", "");
  return name;
}

/** Get the leaf segment of a folder name for display in nested context */
function getLeafName(folder: FolderType): string {
  if (folder.display_name) return folder.display_name;
  const delim = folder.delimiter || "/";
  const parts = folder.name.split(delim);
  return parts[parts.length - 1];
}

/** Build a tree from flat folder list using delimiter-based hierarchy */
function buildTree(folders: FolderType[]): FolderNode[] {
  // Separate special folders (flat) from hierarchical ones
  const special: FolderType[] = [];
  const rest: FolderType[] = [];

  for (const f of folders) {
    if (f.name === "[Gmail]") continue; // skip container
    if (f.folder_type && SORT_ORDER.includes(f.folder_type)) {
      special.push(f);
    } else {
      rest.push(f);
    }
  }

  // Sort special folders by known order
  special.sort((a, b) => {
    const ai = SORT_ORDER.indexOf(a.folder_type || "");
    const bi = SORT_ORDER.indexOf(b.folder_type || "");
    return ai - bi;
  });

  // Build tree from remaining folders
  const rootMap = new Map<string, FolderNode>();

  // Sort rest alphabetically
  rest.sort((a, b) => a.name.localeCompare(b.name));

  for (const f of rest) {
    const delim = f.delimiter || "/";
    const parts = f.name.split(delim);

    let currentMap = rootMap;
    let pathSoFar = "";

    for (let i = 0; i < parts.length; i++) {
      const part = parts[i];
      pathSoFar = pathSoFar ? `${pathSoFar}${delim}${part}` : part;
      const isLast = i === parts.length - 1;

      let existing = currentMap.get(part);
      if (!existing) {
        existing = {
          folder: isLast ? f : null,
          label: part,
          children: [],
        };
        currentMap.set(part, existing);
      } else if (isLast) {
        existing.folder = f;
      }

      // Convert children array to a map for next level
      if (!isLast) {
        const childMap = new Map<string, FolderNode>();
        for (const child of existing.children) {
          childMap.set(child.label, child);
        }
        currentMap = childMap;
        // Sync back
        existing.children = Array.from(childMap.values());
      }
    }
  }

  // Combine: special folders first (flat), then tree nodes
  const result: FolderNode[] = special.map((f) => ({
    folder: f,
    label: getFolderDisplayName(f),
    children: [],
  }));

  // Add tree roots, syncing map back to arrays
  const syncNode = (map: Map<string, FolderNode>): FolderNode[] => {
    return Array.from(map.values());
  };

  result.push(...syncNode(rootMap));
  return result;
}

/** Protected folder types that cannot be renamed or deleted */
const PROTECTED_TYPES = new Set(SORT_ORDER);

interface FolderTreeProps {
  folders: FolderType[];
  accountId: string;
  accentColor?: string;
}

export default function FolderTree({ folders, accountId, accentColor }: FolderTreeProps) {
  const { selectedFolder, selectedAccountId, setSelectedFolder, setAccountFolders } = useMailStore();
  const [collapsed, setCollapsed] = useState<Set<string>>(() => new Set());
  const [creating, setCreating] = useState<string | null>(null); // parent path or "" for root
  const [newName, setNewName] = useState("");
  const [renaming, setRenaming] = useState<string | null>(null); // folder.name being renamed
  const [renameName, setRenameName] = useState("");
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number; folder: FolderType } | null>(null);
  const createInputRef = useRef<HTMLInputElement>(null);
  const renameInputRef = useRef<HTMLInputElement>(null);
  const isThisAccount = selectedAccountId === accountId;

  const toggleCollapse = useCallback((key: string) => {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }, []);

  // Focus inputs when they appear
  useEffect(() => {
    if (creating !== null) createInputRef.current?.focus();
  }, [creating]);
  useEffect(() => {
    if (renaming !== null) {
      renameInputRef.current?.focus();
      renameInputRef.current?.select();
    }
  }, [renaming]);

  // Close context menu on click outside
  useEffect(() => {
    if (!contextMenu) return;
    const close = () => setContextMenu(null);
    window.addEventListener("click", close);
    return () => window.removeEventListener("click", close);
  }, [contextMenu]);

  const handleCreate = async () => {
    const name = newName.trim();
    if (!name) { setCreating(null); return; }

    // Build full folder name with parent prefix
    const delimiter = folders[0]?.delimiter || "/";
    const fullName = creating ? `${creating}${delimiter}${name}` : name;

    try {
      const updated = await api.folders.create(accountId, fullName);
      setAccountFolders(accountId, updated);
    } catch (e) {
      console.error("Failed to create folder:", e);
    }
    setCreating(null);
    setNewName("");
  };

  const handleRename = async () => {
    const name = renameName.trim();
    if (!name || !renaming) { setRenaming(null); return; }

    // Build new full path: replace the last segment
    const delimiter = folders.find((f) => f.name === renaming)?.delimiter || "/";
    const parts = renaming.split(delimiter);
    parts[parts.length - 1] = name;
    const newFullName = parts.join(delimiter);

    if (newFullName === renaming) { setRenaming(null); return; }

    try {
      const updated = await api.folders.rename(accountId, renaming, newFullName);
      setAccountFolders(accountId, updated);
    } catch (e) {
      console.error("Failed to rename folder:", e);
    }
    setRenaming(null);
    setRenameName("");
  };

  const handleDelete = async (folderName: string) => {
    try {
      const updated = await api.folders.delete(accountId, folderName);
      setAccountFolders(accountId, updated);
    } catch (e) {
      console.error("Failed to delete folder:", e);
    }
    setContextMenu(null);
  };

  const tree = buildTree(folders);

  const renderCreateInput = () => (
    <div className="flex items-center gap-1 pl-4 pr-2 py-1">
      <span className="w-4 shrink-0" />
      <Folder className="h-4 w-4 shrink-0 text-content-muted" />
      <input
        ref={createInputRef}
        value={newName}
        onChange={(e) => setNewName(e.target.value)}
        onBlur={handleCreate}
        onKeyDown={(e) => {
          if (e.key === "Enter") handleCreate();
          if (e.key === "Escape") { setCreating(null); setNewName(""); }
        }}
        className="flex-1 rounded bg-elevated px-1.5 py-0.5 text-sm text-content outline-none focus:ring-1 focus:ring-accent"
        placeholder="Folder name"
      />
    </div>
  );

  const renderNode = (node: FolderNode, depth: number, parentKey: string) => {
    const hasChildren = node.children.length > 0;
    const nodeKey = parentKey ? `${parentKey}/${node.label}` : node.label;
    const isCollapsed = collapsed.has(nodeKey);
    const folder = node.folder;
    const isSelected = folder && isThisAccount && selectedFolder === folder.name;
    const isProtected = folder && folder.folder_type && PROTECTED_TYPES.has(folder.folder_type);
    const Icon = folder ? (FOLDER_ICONS[folder.folder_type || ""] || Folder) : Folder;
    const displayName = folder ? (depth > 0 ? getLeafName(folder) : getFolderDisplayName(folder)) : node.label;
    const isRenamingThis = folder && renaming === folder.name;

    return (
      <div key={nodeKey}>
        <div className="group flex items-center">
          {/* Chevron for collapsible parents */}
          {hasChildren ? (
            <button
              onClick={() => toggleCollapse(nodeKey)}
              className="flex h-5 w-4 shrink-0 items-center justify-center text-content-muted hover:text-content"
            >
              <ChevronRight
                className={cn(
                  "h-3 w-3 transition-transform duration-150",
                  !isCollapsed && "rotate-90"
                )}
              />
            </button>
          ) : (
            <span className="w-4 shrink-0" />
          )}
          {isRenamingThis ? (
            <div className="flex flex-1 items-center gap-2 px-2 py-1">
              <Icon className="h-4 w-4 shrink-0" />
              <input
                ref={renameInputRef}
                value={renameName}
                onChange={(e) => setRenameName(e.target.value)}
                onBlur={handleRename}
                onKeyDown={(e) => {
                  if (e.key === "Enter") handleRename();
                  if (e.key === "Escape") { setRenaming(null); setRenameName(""); }
                }}
                className="flex-1 rounded bg-elevated px-1.5 py-0.5 text-sm text-content outline-none focus:ring-1 focus:ring-accent"
              />
            </div>
          ) : (
            <button
              onClick={() => folder && setSelectedFolder(folder.name, accountId)}
              onContextMenu={(e) => {
                if (folder && !isProtected) {
                  e.preventDefault();
                  setContextMenu({ x: e.clientX, y: e.clientY, folder });
                }
              }}
              className={cn(
                "sidebar-item flex flex-1 items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm transition-colors",
                isSelected
                  ? "text-content"
                  : folder
                    ? "text-content-secondary hover:bg-surface"
                    : "text-content-muted"
              )}
              style={isSelected ? { backgroundColor: `${accentColor || "#0a84ff"}26`, color: accentColor || "#0a84ff" } : undefined}
              disabled={!folder}
            >
              <Icon className="h-4 w-4 shrink-0" />
              <span className="flex-1 truncate">{displayName}</span>
              {folder && folder.unread_count > 0 && (
                <span className="text-xs font-medium text-content-secondary">
                  {folder.unread_count}
                </span>
              )}
            </button>
          )}
        </div>
        {hasChildren && !isCollapsed && (
          <div className="pl-4">
            {node.children.map((child) => renderNode(child, depth + 1, nodeKey))}
          </div>
        )}
        {/* Show create input under this node if it's the target parent */}
        {creating !== null && folder && creating === folder.name && renderCreateInput()}
      </div>
    );
  };

  return (
    <div className="space-y-0.5 pl-3 pr-2">
      {tree.map((node) => renderNode(node, 0, ""))}

      {/* Create input at root level */}
      {creating === "" && renderCreateInput()}

      {/* Add folder button */}
      {creating === null && (
        <button
          onClick={() => { setCreating(""); setNewName(""); }}
          className="flex w-full items-center gap-2 rounded-md px-2 py-1 text-left text-xs text-content-muted hover:text-content-secondary transition-colors"
        >
          <span className="w-4 shrink-0" />
          <Plus className="h-3 w-3 shrink-0" />
          <span>New folder</span>
        </button>
      )}

      {/* Context menu */}
      {contextMenu && (
        <div
          className="fixed z-50 min-w-[140px] rounded-md border border-border bg-elevated py-1 shadow-lg"
          style={{ left: contextMenu.x, top: contextMenu.y }}
        >
          <button
            onClick={() => {
              if (contextMenu.folder) {
                setCreating(contextMenu.folder.name);
                setNewName("");
                // Expand the parent so the input is visible
                const delim = contextMenu.folder.delimiter || "/";
                const parts = contextMenu.folder.name.split(delim);
                // Ensure the parent node is expanded in our collapsed set
                setCollapsed((prev) => {
                  const next = new Set(prev);
                  // Remove any collapsed state for paths leading to this folder
                  let path = "";
                  for (const part of parts) {
                    path = path ? `${path}/${part}` : part;
                    next.delete(path);
                  }
                  return next;
                });
              }
              setContextMenu(null);
            }}
            className="flex w-full items-center gap-2 px-3 py-1.5 text-sm text-content-secondary hover:bg-surface hover:text-content"
          >
            <Plus className="h-3.5 w-3.5" />
            New subfolder
          </button>
          <button
            onClick={() => {
              if (contextMenu.folder) {
                const delim = contextMenu.folder.delimiter || "/";
                const parts = contextMenu.folder.name.split(delim);
                setRenameName(parts[parts.length - 1]);
                setRenaming(contextMenu.folder.name);
              }
              setContextMenu(null);
            }}
            className="flex w-full items-center gap-2 px-3 py-1.5 text-sm text-content-secondary hover:bg-surface hover:text-content"
          >
            <Pencil className="h-3.5 w-3.5" />
            Rename
          </button>
          <div className="my-1 border-t border-border" />
          <button
            onClick={() => handleDelete(contextMenu.folder.name)}
            className="flex w-full items-center gap-2 px-3 py-1.5 text-sm text-red-400 hover:bg-surface hover:text-red-300"
          >
            <Trash2 className="h-3.5 w-3.5" />
            Delete
          </button>
        </div>
      )}
    </div>
  );
}
