--- Highlight group definitions.
local M = {}

--- Default opacity for background highlights (0-1)
M.bg_opacity = 0.25

--- Blend two colors with a given alpha.
--- @param fg string Foreground hex color (e.g., "#ff0000")
--- @param bg string Background hex color
--- @param alpha number Blend factor (0 = all bg, 1 = all fg)
--- @return string Blended hex color
local function blend(fg, bg, alpha)
    local function hex_to_rgb(hex)
        hex = hex:gsub("#", "")
        return tonumber(hex:sub(1, 2), 16), tonumber(hex:sub(3, 4), 16), tonumber(hex:sub(5, 6), 16)
    end

    local fg_r, fg_g, fg_b = hex_to_rgb(fg)
    local bg_r, bg_g, bg_b = hex_to_rgb(bg)

    local r = math.floor(fg_r * alpha + bg_r * (1 - alpha))
    local g = math.floor(fg_g * alpha + bg_g * (1 - alpha))
    local b = math.floor(fg_b * alpha + bg_b * (1 - alpha))

    return string.format("#%02x%02x%02x", r, g, b)
end

--- Get the foreground color from a highlight group.
--- @param name string Highlight group name
--- @return string|nil Hex color or nil
local function get_fg(name)
    local hl = vim.api.nvim_get_hl(0, { name = name, link = false })
    if hl.fg then
        return string.format("#%06x", hl.fg)
    end
    return nil
end

--- Get the background color from Normal or fallback.
--- @return string Hex color
local function get_normal_bg()
    local hl = vim.api.nvim_get_hl(0, { name = "Normal", link = false })
    if hl.bg then
        return string.format("#%06x", hl.bg)
    end
    return "#1a1b26" -- fallback dark background
end

--- Linked highlight definitions (inherit from colorscheme)
--- @type table<string, vim.api.keyset.highlight>
M.linked = {
    -- Foreground highlights (link to semantic groups)
    DifftAddedFg = { link = "Added" },
    DifftRemovedFg = { link = "Removed" },

    -- Tree highlights
    DifftFileAdded = { link = "Added" },
    DifftFileDeleted = { link = "Removed" },
    DifftDirectory = { link = "Directory" },

    -- Picker text highlights
    DifftPickerJjIconCurrent = { link = "Added" },
    DifftPickerJjIconImmutable = { link = "Removed" },
    DifftPickerJjIconNormal = { link = "Directory" },
    DifftPickerJjRevset = { link = "Identifier" },
    DifftPickerJjAge = { link = "Comment" },
}


--- Apply all highlight groups.
--- @param overrides table<string, vim.api.keyset.highlight> User overrides
local function apply_highlights(overrides)
    -- Setup linked highlights
    for name, default in pairs(M.linked) do
        local hl = vim.tbl_extend("force", default, overrides[name] or {})
        vim.api.nvim_set_hl(0, name, hl)
    end

    -- Setup derived highlights
    local normal_bg = get_normal_bg()
    local normal_fg = get_fg("Normal") or "#c0caf5"
    local added_fg = get_fg("Added") or "#9ece6a"
    local removed_fg = get_fg("Removed") or "#f7768e"

    local added_bg = blend(added_fg, normal_bg, M.bg_opacity)
    local removed_bg = blend(removed_fg, normal_bg, M.bg_opacity)
    local normal_blend = blend(normal_fg, normal_bg, M.bg_opacity)

    local derived = {
        -- Background highlights (blended from fg colors)
        DifftAdded = { bg = added_bg },
        DifftRemoved = { bg = removed_bg },
        DifftTreeCurrent = { bg = normal_blend, bold = true },
        DifftPickerPreviewHover = { bg = normal_blend, bold = true },
        DifftPickerJjDesc = { fg = normal_fg },
        -- Foreground highlights
        DifftAddedInlineFg = { fg = added_fg, bold = true },
        DifftRemovedInlineFg = { fg = removed_fg, bold = true },
        DifftFiller = { fg = normal_blend },
    }

    for name, default in pairs(derived) do
        local hl = vim.tbl_extend("force", default, overrides[name] or {})
        vim.api.nvim_set_hl(0, name, hl)
    end
end

--- Setup highlight groups with optional overrides.
--- @param overrides table<string, vim.api.keyset.highlight>|nil User overrides
function M.setup(overrides)
    overrides = overrides or {}

    -- Apply highlights now
    apply_highlights(overrides)

    -- Reapply on colorscheme change
    vim.api.nvim_create_autocmd("ColorScheme", {
        group = vim.api.nvim_create_augroup("DifftHighlights", { clear = true }),
        callback = function()
            apply_highlights(overrides)
        end,
    })
end

return M
