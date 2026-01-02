--- File tree sidebar using nui.nvim.
local M = {}

local NuiTree = require("nui.tree")
local NuiLine = require("nui.line")

local DEFAULT_ICON = ""
local has_devicons, devicons = pcall(require, "nvim-web-devicons")

--- Module state
--- @type table|nil
M.tree = nil
--- @type table<number, string>
M.file_to_node_id = {}
--- @type number|nil
M.current_file_idx = nil
--- @type number
M.total_additions = 0
--- @type number
M.total_deletions = 0
--- Number of header lines (blank + summary + blank)
M.header_lines = 3

--- @return table Tree configuration
local function get_config()
    return require("difftastic-nvim").config.tree
end

--- Get file icon from nvim-web-devicons if available.
--- @param filename string
--- @return string icon
--- @return string|nil highlight_group
local function get_file_icon(filename)
    local cfg = get_config()
    if cfg.icons.enable and has_devicons then
        local icon, hl = devicons.get_icon(filename, nil, { default = true })
        return icon or DEFAULT_ICON, hl
    end
    return DEFAULT_ICON, nil
end

--- Build an intermediate tree structure from flat file list.
--- @param files table[] List of file objects with path, status, additions, deletions
--- @return table Root node of the tree
local function build_intermediate_tree(files)
    local root = {
        name = "",
        path = "",
        is_dir = true,
        children = {},
        children_map = {},
        file_idx = nil,
        status = nil,
        additions = 0,
        deletions = 0,
    }

    for idx, file in ipairs(files) do
        local parts = {}
        for part in string.gmatch(file.path, "[^/]+") do
            table.insert(parts, part)
        end

        local node = root
        local current_path = ""
        for i, part in ipairs(parts) do
            local is_last = (i == #parts)
            current_path = current_path == "" and part or (current_path .. "/" .. part)

            if not node.children_map[part] then
                local child = {
                    name = part,
                    path = current_path,
                    is_dir = not is_last,
                    children = {},
                    children_map = {},
                    file_idx = nil,
                    status = nil,
                    additions = 0,
                    deletions = 0,
                }
                node.children_map[part] = child
                table.insert(node.children, child)
            end

            node = node.children_map[part]

            if is_last then
                node.file_idx = idx
                node.status = file.status
                node.additions = file.additions or 0
                node.deletions = file.deletions or 0
            end
        end
    end

    return root
end

local function propagate_stats(node)
    if not node.is_dir then
        return node.additions, node.deletions
    end

    local total_add, total_del = 0, 0
    for _, child in ipairs(node.children) do
        local add, del = propagate_stats(child)
        total_add = total_add + add
        total_del = total_del + del
    end

    node.additions = total_add
    node.deletions = total_del
    return total_add, total_del
end

local function flatten_node(node)
    for _, child in ipairs(node.children) do
        flatten_node(child)
    end

    while #node.children == 1 and node.children[1].is_dir do
        local child = node.children[1]
        node.name = node.name == "" and child.name or (node.name .. "/" .. child.name)
        node.path = child.path
        node.children = child.children
        node.children_map = child.children_map
    end
end

local function sort_node(node)
    table.sort(node.children, function(a, b)
        if a.is_dir ~= b.is_dir then return a.is_dir end
        return a.name:lower() < b.name:lower()
    end)

    for _, child in ipairs(node.children) do
        if child.is_dir then sort_node(child) end
    end
end

local function convert_to_nui_nodes(node, file_to_node_id)
    local nui_children = {}

    for _, child in ipairs(node.children) do
        local grandchildren = nil
        if child.is_dir then
            grandchildren = convert_to_nui_nodes(child, file_to_node_id)
        end

        local nui_node = NuiTree.Node({
            id = child.path,
            name = child.name,
            path = child.path,
            is_dir = child.is_dir,
            file_idx = child.file_idx,
            status = child.status,
            additions = child.additions,
            deletions = child.deletions,
        }, grandchildren)

        if child.file_idx then
            file_to_node_id[child.file_idx] = child.path
        end

        if child.is_dir then
            nui_node:expand()
        end

        table.insert(nui_children, nui_node)
    end

    return nui_children
end

local function prepare_node(node)
    local cfg = get_config()
    local line = NuiLine()
    local depth = node:get_depth()

    -- Indentation
    line:append(string.rep("  ", depth - 1))

    -- Icon
    local icon, icon_hl
    if node.is_dir then
        icon = node:is_expanded() and cfg.icons.dir_open or cfg.icons.dir_closed
        icon_hl = "DifftDirectory"
    else
        icon, icon_hl = get_file_icon(node.name)
    end
    line:append(icon .. " ", icon_hl)

    -- Name
    line:append(node.name)

    -- Stats
    if node.additions > 0 or node.deletions > 0 then
        line:append(" ")
        if node.additions > 0 then
            line:append("+" .. node.additions, "DifftFileAdded")
            if node.deletions > 0 then
                line:append(" ")
            end
        end
        if node.deletions > 0 then
            line:append("-" .. node.deletions, "DifftFileDeleted")
        end
    end

    return line
end

--- Render the header with totals.
--- @param state table Plugin state
--- @param total_add number Total additions
--- @param total_del number Total deletions
local function render_header(state, total_add, total_del)
    local width = get_config().width

    -- Format: "Additions: N | Deletions: M"
    local add_label, del_label = "Additions: ", "Deletions: "
    local separator = " | "
    local stats = add_label .. total_add .. separator .. del_label .. total_del

    -- Center the stats
    local padding = math.floor((width - #stats) / 2)
    local header_line = string.rep(" ", math.max(0, padding)) .. stats

    vim.api.nvim_buf_set_lines(state.tree_buf, 0, 0, false, { "", header_line, "" })

    -- Apply highlights to the numbers only (on line index 1, the middle line)
    local ns = vim.api.nvim_create_namespace("difft-tree-header")
    local add_num_start = padding + #add_label
    local add_num_end = add_num_start + #tostring(total_add)
    vim.api.nvim_buf_add_highlight(state.tree_buf, ns, "DifftFileAdded", 1, add_num_start, add_num_end)

    local del_num_start = padding + #add_label + #tostring(total_add) + #separator + #del_label
    local del_num_end = del_num_start + #tostring(total_del)
    vim.api.nvim_buf_add_highlight(state.tree_buf, ns, "DifftFileDeleted", 1, del_num_start, del_num_end)
end

function M.open(state)
    vim.cmd("topleft vertical " .. get_config().width .. " new")
    state.tree_win = vim.api.nvim_get_current_win()
    state.tree_buf = vim.api.nvim_get_current_buf()

    vim.wo[state.tree_win].number = false
    vim.wo[state.tree_win].relativenumber = false
    vim.wo[state.tree_win].signcolumn = "no"
    vim.wo[state.tree_win].winfixwidth = true
    vim.wo[state.tree_win].cursorline = true
    vim.wo[state.tree_win].scrollbind = false
    vim.wo[state.tree_win].cursorbind = false

    vim.bo[state.tree_buf].buftype = "nofile"
    vim.bo[state.tree_buf].bufhidden = "wipe"
    vim.bo[state.tree_buf].swapfile = false
    vim.bo[state.tree_buf].filetype = "difft-tree"
    vim.bo[state.tree_buf].modifiable = true

    -- Build intermediate tree structure
    local root = build_intermediate_tree(state.files)
    propagate_stats(root)
    flatten_node(root)
    sort_node(root)

    -- Store totals for header
    M.total_additions = root.additions
    M.total_deletions = root.deletions

    -- Convert to nui nodes
    M.file_to_node_id = {}
    local nui_nodes = convert_to_nui_nodes(root, M.file_to_node_id)

    -- Render header first
    render_header(state, root.additions, root.deletions)

    -- Create nui tree (starts after header)
    M.tree = NuiTree({
        bufnr = state.tree_buf,
        nodes = nui_nodes,
        prepare_node = prepare_node,
    })

    M.tree:render(M.header_lines + 1)

    -- Keymaps
    local difft = require("difftastic-nvim")
    local keys = difft.config.keymaps

    vim.keymap.set("n", keys.select, function()
        local node = M.tree:get_node()
        if not node then return end

        if node.file_idx then
            difft.show_file(node.file_idx)
        elseif node.is_dir then
            if node:is_expanded() then
                node:collapse()
            else
                node:expand()
            end
            M.tree:render()
        end
    end, { buffer = state.tree_buf })

    vim.keymap.set("n", keys.close, difft.close, { buffer = state.tree_buf })
end

function M.render(state)
    if M.tree then
        M.tree:render()
    end
end

local function collect_visible_files(tree)
    local files = {}

    local function walk(node_id)
        local nodes = tree:get_nodes(node_id)
        for _, node in ipairs(nodes) do
            if node.file_idx then
                table.insert(files, node.file_idx)
            end
            if node:has_children() and node:is_expanded() then
                walk(node:get_id())
            end
        end
    end

    walk()
    return files
end

function M.next_file_in_display_order(current_idx)
    if not M.tree then return nil end
    local files = collect_visible_files(M.tree)
    for i, idx in ipairs(files) do
        if idx == current_idx and files[i + 1] then
            return files[i + 1]
        end
    end
    return nil
end

function M.prev_file_in_display_order(current_idx)
    if not M.tree then return nil end
    local files = collect_visible_files(M.tree)
    for i, idx in ipairs(files) do
        if idx == current_idx and i > 1 then
            return files[i - 1]
        end
    end
    return nil
end

function M.first_file_in_display_order()
    if not M.tree then return nil end
    local files = collect_visible_files(M.tree)
    return files[1]
end

function M.last_file_in_display_order()
    if not M.tree then return nil end
    local files = collect_visible_files(M.tree)
    return files[#files]
end

function M.highlight_current(state)
    if not M.tree or not state.tree_buf then return end

    local ns = vim.api.nvim_create_namespace("difft-tree-current")
    vim.api.nvim_buf_clear_namespace(state.tree_buf, ns, M.header_lines, -1)

    M.current_file_idx = state.current_file_idx

    -- Find the line number by iterating through rendered lines (after header)
    local line_count = vim.api.nvim_buf_line_count(state.tree_buf)
    for linenr = M.header_lines + 1, line_count do
        local node = M.tree:get_node(linenr)
        if node and node.file_idx == state.current_file_idx then
            vim.api.nvim_buf_add_highlight(state.tree_buf, ns, "DifftTreeCurrent", linenr - 1, 0, -1)
            if vim.api.nvim_win_is_valid(state.tree_win) then
                vim.api.nvim_win_set_cursor(state.tree_win, { linenr, 0 })
            end
            break
        end
    end
end

return M
