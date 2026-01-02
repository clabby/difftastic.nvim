--- Tests for diff.lua hunk navigation functions
local diff = require("difftastic-nvim.diff")

describe("diff", function()
    local mock_win
    local mock_cursor_line

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
