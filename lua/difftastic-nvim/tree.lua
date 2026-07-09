--- File tree sidebar using nui.nvim.
local M = {}

local NuiTree = require("nui.tree")
local NuiLine = require("nui.line")

local DEFAULT_ICON = ""
local has_devicons, devicons = pcall(require, "nvim-web-devicons")

local GLYPHS = {
    branch = "│ ",
    expanded = "",
    collapsed = "",
    file = "  ",
    added = "+",
    deleted = "-",
    changed = "●",
    renamed = "➜",
}

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
--- Number of header lines (title + summary + range + divider)
M.header_lines = 4

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

local function status_icon(node)
    if node.moved_from then
        return GLYPHS.renamed, "DifftTreeRenamed"
    end
    if node.status == "created" then
        return GLYPHS.added, "DifftTreeAdded"
    end
    if node.status == "deleted" then
        return GLYPHS.deleted, "DifftTreeDeleted"
    end
    if node.additions > 0 or node.deletions > 0 then
        return GLYPHS.changed, "DifftTreeModified"
    end
    return " ", "DifftTreeMuted"
end

local function append_stat_chip(line, additions, deletions)
    if additions == 0 and deletions == 0 then
        return
    end

    line:append("  ", "DifftTreeMuted")
    if additions > 0 then
        line:append("+" .. additions, "DifftFileAdded")
    end
    if additions > 0 and deletions > 0 then
        line:append(" ", "DifftTreeMuted")
    end
    if deletions > 0 then
        line:append("-" .. deletions, "DifftFileDeleted")
    end
end

local function display_width(text)
    return vim.fn.strdisplaywidth(text)
end

local function pad_to_width(text, width)
    return text .. string.rep(" ", math.max(0, width - display_width(text)))
end

local function trim_to_width(text, width)
    if width <= 0 then
        return ""
    end

    if display_width(text) <= width then
        return text
    end

    local ellipsis = "…"
    local limit = math.max(0, width - display_width(ellipsis))
    local result = vim.fn.strcharpart(text, 0, limit)

    while display_width(result .. ellipsis) > width do
        result = vim.fn.strcharpart(result, 0, math.max(0, vim.fn.strchars(result) - 1))
    end

    return result .. ellipsis
end

local function fit_header_row(left, right, width)
    local gap = width - display_width(left) - display_width(right)
    if gap < 1 then
        local right_width = math.max(0, width - display_width(left) - 1)
        if right_width > 0 then
            return fit_header_row(left, trim_to_width(right, right_width), width)
        end
        return pad_to_width(trim_to_width(left, width), width)
    end

    return left .. string.rep(" ", gap) .. right
end

local function header_border(width, title)
    local title_text = title and (" " .. title .. " ") or ""
    local inner_width = math.max(0, width - 2)
    local title_width = display_width(title_text)

    if title_width == 0 or title_width >= inner_width then
        return "╭" .. string.rep("─", inner_width) .. "╮"
    end

    local left = math.floor((inner_width - title_width) / 2)
    local right = inner_width - title_width - left
    return "╭" .. string.rep("─", left) .. title_text .. string.rep("─", right) .. "╮"
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
                node.moved_from = file.moved_from
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
            moved_from = child.moved_from,
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

    for _ = 1, depth - 1 do
        line:append(GLYPHS.branch, "DifftTreeIndent")
    end

    if node.is_dir then
        line:append(node:is_expanded() and GLYPHS.expanded or GLYPHS.collapsed, "DifftTreeChevron")
        line:append(" ", "DifftTreeMuted")
    else
        line:append(GLYPHS.file, "DifftTreeMuted")
    end

    local marker, marker_hl = status_icon(node)
    line:append(marker .. " ", marker_hl)

    local icon, icon_hl
    if node.is_dir then
        icon = node:is_expanded() and cfg.icons.dir_open or cfg.icons.dir_closed
        icon_hl = "DifftDirectory"
    else
        icon, icon_hl = get_file_icon(node.name)
    end
    line:append(icon .. " ", icon_hl)

    if node.moved_from then
        line:append(node.moved_from, "DifftTreePathMuted")
        line:append(" → ", "DifftTreeMuted")
        line:append(node.name, "DifftFileAdded")
    elseif node.is_dir then
        line:append(node.name, "DifftTreeDirectory")
    else
        line:append(node.name, "DifftTreeFile")
    end

    append_stat_chip(line, node.additions, node.deletions)

    return line
end

--- Render the header with totals.
--- @param state table Plugin state
--- @param total_add number Total additions
--- @param total_del number Total deletions
local function render_header(state, total_add, total_del)
    local width = get_config().width

    local ns = vim.api.nvim_create_namespace("difft-tree-header")
    vim.api.nvim_buf_clear_namespace(state.tree_buf, ns, 0, M.header_lines)

    local file_count = #(state.files or {})
    local file_label = file_count == 1 and "1 file" or (file_count .. " files")

    local add_text = "+" .. total_add
    local del_text = "-" .. total_del
    local stat_text = add_text .. "  " .. del_text
    local inner_width = math.max(0, width - 4)
    local stats_inner = fit_header_row(file_label, stat_text, inner_width)
    local range_kind = state.range_kind or "Range"
    local range_text = state.range_label or ""
    local range_value_width = math.max(0, inner_width - display_width(range_kind) - 1)
    local range_display = range_value_width > 0 and trim_to_width(range_text, range_value_width) or ""
    local range_inner = fit_header_row(range_kind, range_display, inner_width)

    local top_line = header_border(width, "Difftastic")
    local stats_line = "│ " .. stats_inner .. " │"
    local range_line = "│ " .. range_inner .. " │"
    local bottom_line = "╰" .. string.rep("─", math.max(0, width - 2)) .. "╯"

    vim.api.nvim_buf_set_lines(state.tree_buf, 0, 0, false, { top_line, stats_line, range_line, bottom_line })

    local title_start = top_line:find("Difftastic", 1, true)
    if title_start then
        vim.api.nvim_buf_add_highlight(state.tree_buf, ns, "DifftTreeDivider", 0, 0, -1)
        vim.api.nvim_buf_add_highlight(state.tree_buf, ns, "DifftTreeTitle", 0, title_start - 1, title_start + #"Difftastic" - 1)
    end

    local left_border_end = #"│"
    local content_start = #"│ "
    local right_border_start = #stats_line - #"│"

    vim.api.nvim_buf_add_highlight(state.tree_buf, ns, "DifftTreeDivider", 1, 0, left_border_end)
    local file_label_col = stats_line:find(file_label, 1, true)
    if file_label_col then
        vim.api.nvim_buf_add_highlight(state.tree_buf, ns, "DifftTreeMuted", 1, file_label_col - 1, file_label_col + #file_label - 1)
    end
    local add_col = stats_line:find(add_text, 1, true)
    if add_col then
        vim.api.nvim_buf_add_highlight(state.tree_buf, ns, "DifftFileAdded", 1, add_col - 1, add_col + #add_text - 1)
    end
    local del_col = stats_line:find(del_text, add_col and (add_col + #add_text) or 1, true)
    if del_col then
        vim.api.nvim_buf_add_highlight(state.tree_buf, ns, "DifftFileDeleted", 1, del_col - 1, del_col + #del_text - 1)
    end
    vim.api.nvim_buf_add_highlight(state.tree_buf, ns, "DifftTreeDivider", 1, right_border_start, -1)
    local range_right_border_start = #range_line - #"│"
    vim.api.nvim_buf_add_highlight(state.tree_buf, ns, "DifftTreeDivider", 2, 0, left_border_end)
    local range_kind_col = range_line:find(range_kind, 1, true)
    if range_kind_col then
        vim.api.nvim_buf_add_highlight(state.tree_buf, ns, "DifftTreeMuted", 2, range_kind_col - 1, range_kind_col + #range_kind - 1)
    end
    local range_value_col = range_display ~= "" and range_line:find(range_display, 1, true) or nil
    if range_value_col then
        vim.api.nvim_buf_add_highlight(state.tree_buf, ns, "DifftTreeRange", 2, range_value_col - 1, #range_line - #" │")
    end
    vim.api.nvim_buf_add_highlight(state.tree_buf, ns, "DifftTreeDivider", 2, range_right_border_start, -1)
    vim.api.nvim_buf_add_highlight(state.tree_buf, ns, "DifftTreeDivider", 3, 0, -1)
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
    vim.wo[state.tree_win].foldcolumn = "0"
    vim.wo[state.tree_win].list = false
    vim.wo[state.tree_win].winhl = table.concat({
        "Normal:DifftTreeNormal",
        "NormalNC:DifftTreeNormal",
        "EndOfBuffer:DifftTreeEndOfBuffer",
        "CursorLine:DifftTreeCursorLine",
    }, ",")

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
