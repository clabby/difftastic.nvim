--- Tests for diff.lua hunk navigation functions
local diff = require("difftastic-nvim.diff")

describe("diff", function()
    local mock_win
    local mock_cursor_line

    describe("render", function()
        local left_win
        local left_buf
        local right_win
        local right_buf

        before_each(function()
            package.loaded["difftastic-nvim"] = {
                config = {
                    highlight_mode = "treesitter",
                },
            }

            vim.cmd("enew")
            left_win = vim.api.nvim_get_current_win()
            left_buf = vim.api.nvim_get_current_buf()

            vim.cmd("vsplit")
            right_win = vim.api.nvim_get_current_win()
            right_buf = vim.api.nvim_get_current_buf()
        end)

        after_each(function()
            vim.cmd("only")
            pcall(vim.api.nvim_buf_delete, left_buf, { force = true })
            pcall(vim.api.nvim_buf_delete, right_buf, { force = true })
        end)

        it("uses muted line highlights under stronger partial change highlights", function()
            local state = {
                left_win = left_win,
                left_buf = left_buf,
                right_win = right_win,
                right_buf = right_buf,
            }
            local file = {
                language = "Lua",
                hunk_starts = { 0 },
                rows = {
                    {
                        left = {
                            content = "local old_value = 1",
                            is_filler = false,
                            highlights = { { start = 6, ["end"] = 15 } },
                        },
                        right = {
                            content = "local new_value = 1",
                            is_filler = false,
                            highlights = { { start = 6, ["end"] = 15 } },
                        },
                    },
                },
            }

            diff.render(state, file)

            local left_marks = vim.api.nvim_buf_get_extmarks(left_buf, -1, 0, -1, { details = true })
            local right_marks = vim.api.nvim_buf_get_extmarks(right_buf, -1, 0, -1, { details = true })
            local has_removed_line = false
            local has_added_line = false
            local has_removed_range = false
            local has_added_range = false

            for _, mark in ipairs(left_marks) do
                local col = mark[3]
                local details = mark[4] or {}
                if
                    details.hl_group == "DifftRemovedLine"
                    and col == 0
                    and details.end_row == 1
                    and details.end_col == 0
                    and details.hl_eol
                    and details.priority == 100
                then
                    has_removed_line = true
                end
                if
                    details.hl_group == "DifftRemoved"
                    and col == 6
                    and details.end_col == 15
                    and details.priority == 200
                then
                    has_removed_range = true
                end
            end

            for _, mark in ipairs(right_marks) do
                local col = mark[3]
                local details = mark[4] or {}
                if
                    details.hl_group == "DifftAddedLine"
                    and col == 0
                    and details.end_row == 1
                    and details.end_col == 0
                    and details.hl_eol
                    and details.priority == 100
                then
                    has_added_line = true
                end
                if
                    details.hl_group == "DifftAdded"
                    and col == 6
                    and details.end_col == 15
                    and details.priority == 200
                then
                    has_added_range = true
                end
            end

            assert.is_true(has_removed_line)
            assert.is_true(has_added_line)
            assert.is_true(has_removed_range)
            assert.is_true(has_added_range)
        end)

        it("uses muted line highlights for full-line changes", function()
            local state = {
                left_win = left_win,
                left_buf = left_buf,
                right_win = right_win,
                right_buf = right_buf,
            }
            local file = {
                language = "Lua",
                hunk_starts = { 0 },
                rows = {
                    {
                        left = {
                            content = "old_value",
                            is_filler = false,
                            highlights = { { start = 0, ["end"] = -1 } },
                        },
                        right = {
                            content = "new_value",
                            is_filler = false,
                            highlights = { { start = 0, ["end"] = -1 } },
                        },
                    },
                },
            }

            diff.render(state, file)

            local left_marks = vim.api.nvim_buf_get_extmarks(left_buf, -1, 0, -1, { details = true })
            local right_marks = vim.api.nvim_buf_get_extmarks(right_buf, -1, 0, -1, { details = true })
            local has_removed_line = false
            local has_added_line = false
            local has_removed_range = false
            local has_added_range = false

            for _, mark in ipairs(left_marks) do
                local col = mark[3]
                local details = mark[4] or {}
                if
                    details.hl_group == "DifftRemovedLine"
                    and col == 0
                    and details.end_row == 1
                    and details.end_col == 0
                    and details.hl_eol
                    and details.priority == 100
                then
                    has_removed_line = true
                end
                if details.hl_group == "DifftRemoved" and details.priority == 200 then
                    has_removed_range = true
                end
            end

            for _, mark in ipairs(right_marks) do
                local col = mark[3]
                local details = mark[4] or {}
                if
                    details.hl_group == "DifftAddedLine"
                    and col == 0
                    and details.end_row == 1
                    and details.end_col == 0
                    and details.hl_eol
                    and details.priority == 100
                then
                    has_added_line = true
                end
                if details.hl_group == "DifftAdded" and details.priority == 200 then
                    has_added_range = true
                end
            end

            assert.is_true(has_removed_line)
            assert.is_true(has_added_line)
            assert.is_false(has_removed_range)
            assert.is_false(has_added_range)
        end)

        it("uses muted line highlights when every non-whitespace character changed", function()
            local state = {
                left_win = left_win,
                left_buf = left_buf,
                right_win = right_win,
                right_buf = right_buf,
            }
            local file = {
                language = "Lua",
                hunk_starts = { 0 },
                rows = {
                    {
                        left = {
                            content = "foo bar",
                            is_filler = false,
                            highlights = { { start = 0, ["end"] = 3 }, { start = 4, ["end"] = 7 } },
                        },
                        right = {
                            content = "baz qux",
                            is_filler = false,
                            highlights = { { start = 0, ["end"] = 3 }, { start = 4, ["end"] = 7 } },
                        },
                    },
                },
            }

            diff.render(state, file)

            local left_marks = vim.api.nvim_buf_get_extmarks(left_buf, -1, 0, -1, { details = true })
            local right_marks = vim.api.nvim_buf_get_extmarks(right_buf, -1, 0, -1, { details = true })
            local has_removed_line = false
            local has_added_line = false
            local has_removed_range = false
            local has_added_range = false

            for _, mark in ipairs(left_marks) do
                local col = mark[3]
                local details = mark[4] or {}
                if
                    details.hl_group == "DifftRemovedLine"
                    and col == 0
                    and details.end_row == 1
                    and details.end_col == 0
                    and details.hl_eol
                    and details.priority == 100
                then
                    has_removed_line = true
                end
                if details.hl_group == "DifftRemoved" and details.priority == 200 then
                    has_removed_range = true
                end
            end

            for _, mark in ipairs(right_marks) do
                local col = mark[3]
                local details = mark[4] or {}
                if
                    details.hl_group == "DifftAddedLine"
                    and col == 0
                    and details.end_row == 1
                    and details.end_col == 0
                    and details.hl_eol
                    and details.priority == 100
                then
                    has_added_line = true
                end
                if details.hl_group == "DifftAdded" and details.priority == 200 then
                    has_added_range = true
                end
            end

            assert.is_true(has_removed_line)
            assert.is_true(has_added_line)
            assert.is_false(has_removed_range)
            assert.is_false(has_added_range)
        end)
    end)

    before_each(function()
        -- Reset hunk positions
        diff.hunk_positions = {}
        mock_cursor_line = 1

        -- Mock vim API
        mock_win = 1
        _G.vim = _G.vim or {}
        _G.vim.api = _G.vim.api or {}
        _G.vim.api.nvim_get_current_win = function()
            return mock_win
        end
        _G.vim.api.nvim_win_is_valid = function()
            return true
        end
        _G.vim.api.nvim_win_get_cursor = function()
            return { mock_cursor_line, 0 }
        end
        _G.vim.api.nvim_win_set_cursor = function(_, pos)
            mock_cursor_line = pos[1]
        end
    end)

    describe("next_hunk", function()
        it("returns false when no hunks", function()
            diff.hunk_positions = {}
            local state = { left_win = mock_win, right_win = 2 }
            local result = diff.next_hunk(state)
            assert.is_false(result)
        end)

        it("jumps to next hunk when one exists ahead", function()
            diff.hunk_positions = { 5, 15, 30 }
            mock_cursor_line = 1
            local state = { left_win = mock_win, right_win = 2 }

            local result = diff.next_hunk(state)

            assert.is_true(result)
            assert.equals(5, mock_cursor_line)
        end)

        it("jumps to second hunk when cursor is on first", function()
            diff.hunk_positions = { 5, 15, 30 }
            mock_cursor_line = 5
            local state = { left_win = mock_win, right_win = 2 }

            local result = diff.next_hunk(state)

            assert.is_true(result)
            assert.equals(15, mock_cursor_line)
        end)

        it("returns false when at last hunk", function()
            diff.hunk_positions = { 5, 15, 30 }
            mock_cursor_line = 30
            local state = { left_win = mock_win, right_win = 2 }

            local result = diff.next_hunk(state)

            assert.is_false(result)
            assert.equals(30, mock_cursor_line) -- cursor unchanged
        end)

        it("returns false when past all hunks", function()
            diff.hunk_positions = { 5, 15, 30 }
            mock_cursor_line = 50
            local state = { left_win = mock_win, right_win = 2 }

            local result = diff.next_hunk(state)

            assert.is_false(result)
        end)
    end)

    describe("prev_hunk", function()
        it("returns false when no hunks", function()
            diff.hunk_positions = {}
            local state = { left_win = mock_win, right_win = 2 }
            local result = diff.prev_hunk(state)
            assert.is_false(result)
        end)

        it("jumps to previous hunk when one exists behind", function()
            diff.hunk_positions = { 5, 15, 30 }
            mock_cursor_line = 20
            local state = { left_win = mock_win, right_win = 2 }

            local result = diff.prev_hunk(state)

            assert.is_true(result)
            assert.equals(15, mock_cursor_line)
        end)

        it("jumps to first hunk from second", function()
            diff.hunk_positions = { 5, 15, 30 }
            mock_cursor_line = 15
            local state = { left_win = mock_win, right_win = 2 }

            local result = diff.prev_hunk(state)

            assert.is_true(result)
            assert.equals(5, mock_cursor_line)
        end)

        it("returns false when at first hunk", function()
            diff.hunk_positions = { 5, 15, 30 }
            mock_cursor_line = 5
            local state = { left_win = mock_win, right_win = 2 }

            local result = diff.prev_hunk(state)

            assert.is_false(result)
            assert.equals(5, mock_cursor_line) -- cursor unchanged
        end)

        it("returns false when before all hunks", function()
            diff.hunk_positions = { 5, 15, 30 }
            mock_cursor_line = 1
            local state = { left_win = mock_win, right_win = 2 }

            local result = diff.prev_hunk(state)

            assert.is_false(result)
        end)
    end)

    describe("first_hunk", function()
        it("does nothing when no hunks", function()
            diff.hunk_positions = {}
            mock_cursor_line = 10
            local state = { left_win = mock_win, right_win = 2 }

            diff.first_hunk(state)

            assert.equals(10, mock_cursor_line) -- unchanged
        end)

        it("jumps to first hunk", function()
            diff.hunk_positions = { 5, 15, 30 }
            mock_cursor_line = 25
            local state = { left_win = mock_win, right_win = 2 }

            diff.first_hunk(state)

            assert.equals(5, mock_cursor_line)
        end)
    end)

    describe("last_hunk", function()
        it("does nothing when no hunks", function()
            diff.hunk_positions = {}
            mock_cursor_line = 10
            local state = { left_win = mock_win, right_win = 2 }

            diff.last_hunk(state)

            assert.equals(10, mock_cursor_line) -- unchanged
        end)

        it("jumps to last hunk", function()
            diff.hunk_positions = { 5, 15, 30 }
            mock_cursor_line = 1
            local state = { left_win = mock_win, right_win = 2 }

            diff.last_hunk(state)

            assert.equals(30, mock_cursor_line)
        end)
    end)
end)
