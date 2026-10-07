--- End-to-end tests for renamed files through the native library, against real
--- scratch repositories. Each suite is skipped when the library, difft or the VCS
--- binary is missing.

local binary = require("difftastic-nvim.binary")

local lib_ok, lib = pcall(binary.get)
local has_difft = vim.fn.executable("difft") == 1

--- Lines of one side of a file, without filler rows. difft may align one empty
--- line past the end of the file; it is dropped.
local function side(file, which)
    local lines = {}
    for _, row in ipairs(file.rows) do
        if not row[which].is_filler then
            table.insert(lines, row[which].content)
        end
    end
    if lines[#lines] == "" then
        table.remove(lines)
    end
    return lines
end

--- Files of a diff result keyed by path.
local function by_path(result)
    local files = {}
    for _, file in ipairs(result.files) do
        files[file.path] = file
    end
    return files
end

local function count(result)
    return #result.files
end

local BODY = { "one", "two", "three", "four", "five", "six" }
local EDITED = { "one", "two", "THREE", "four", "five", "six" }

--- Writes lines to a repo-relative path, creating parent directories.
local function write(repo, path, lines)
    local full = repo .. "/" .. path
    vim.fn.mkdir(vim.fn.fnamemodify(full, ":h"), "p")
    vim.fn.writefile(lines, full)
end

local function run(cmd, cwd)
    local result = vim.system(cmd, { text = true, cwd = cwd }):wait()
    assert(result.code == 0, table.concat(cmd, " ") .. ": " .. (result.stderr or ""))
    return vim.trim(result.stdout)
end

--- Shared expectations for a rename of `old` to `new`.
local function assert_renamed(result, old, new, old_lines, new_lines)
    local files = by_path(result)
    local file = files[new]
    assert.is_not_nil(file, "renamed file missing: " .. new)
    assert.is_nil(files[old], "old path still listed: " .. old)
    assert.are.equal(old, file.moved_from)
    assert.are.same(old_lines, side(file, "left"))
    assert.are.same(new_lines, side(file, "right"))
end

local function assert_pure_rename(result, old, new)
    assert_renamed(result, old, new, BODY, BODY)
    local file = by_path(result)[new]
    assert.are.equal(0, file.additions)
    assert.are.equal(0, file.deletions)
    assert.are.same({}, file.hunk_starts)
end

local function assert_edited_rename(result, old, new, with_stats)
    assert_renamed(result, old, new, BODY, EDITED)
    local file = by_path(result)[new]
    assert.are.equal(1, #file.hunk_starts)
    if with_stats then
        assert.are.equal(1, file.additions)
        assert.are.equal(1, file.deletions)
    end
end

describe("git renames", function()
    if not (lib_ok and has_difft and vim.fn.executable("git") == 1) then
        it("is skipped without the native library, difft or git", function() end)
        return
    end

    local repo, original_cwd

    local function git(args)
        local cmd = { "git", "-c", "user.name=t", "-c", "user.email=t@t" }
        vim.list_extend(cmd, args)
        return run(cmd, repo)
    end

    local function commit(message)
        git({ "add", "-A" })
        git({ "commit", "-q", "-m", message })
    end

    before_each(function()
        original_cwd = vim.fn.getcwd()
        repo = vim.fn.resolve(vim.fn.tempname())
        vim.fn.mkdir(repo, "p")
        git({ "init", "-q" })
        write(repo, "a.txt", BODY)
        write(repo, "src/old.txt", BODY)
        write(repo, "keep.txt", { "keep" })
        commit("initial")
        vim.fn.chdir(repo)
    end)

    after_each(function()
        vim.fn.chdir(original_cwd)
        vim.fn.delete(repo, "rf")
    end)

    it("shows a pure rename in a single commit", function()
        git({ "mv", "a.txt", "b.txt" })
        commit("rename")

        local result = lib.run_diff("HEAD", "git")

        assert.are.equal(1, count(result))
        assert_pure_rename(result, "a.txt", "b.txt")
    end)

    it("shows an edited rename in a single commit", function()
        git({ "mv", "a.txt", "b.txt" })
        write(repo, "b.txt", EDITED)
        commit("rename and edit")

        local result = lib.run_diff("HEAD", "git")

        assert.are.equal(1, count(result))
        assert_edited_rename(result, "a.txt", "b.txt", true)
    end)

    it("shows a rename into another directory", function()
        vim.fn.mkdir(repo .. "/dir", "p")
        git({ "mv", "a.txt", "dir/moved.txt" })
        write(repo, "dir/moved.txt", EDITED)
        commit("move")

        local result = lib.run_diff("HEAD", "git")

        assert_edited_rename(result, "a.txt", "dir/moved.txt", true)
    end)

    it("shows a rename within a directory", function()
        git({ "mv", "src/old.txt", "src/new.txt" })
        write(repo, "src/new.txt", EDITED)
        commit("rename in dir")

        local result = lib.run_diff("HEAD", "git")

        assert_edited_rename(result, "src/old.txt", "src/new.txt", true)
    end)

    it("shows a rename inside a range", function()
        write(repo, "keep.txt", { "kept" })
        commit("unrelated")
        git({ "mv", "a.txt", "b.txt" })
        write(repo, "b.txt", EDITED)
        commit("rename")

        local result = lib.run_diff("HEAD~2..HEAD", "git")

        assert.are.equal(2, count(result))
        assert_edited_rename(result, "a.txt", "b.txt", true)
    end)

    it("ignores a rename the working tree adds on top of the commit", function()
        write(repo, "keep.txt", { "kept" })
        commit("unrelated")
        git({ "mv", "a.txt", "b.txt" })

        local result = lib.run_diff("HEAD", "git")

        assert.are.equal(1, count(result))
        assert.are.equal("keep.txt", result.files[1].path)
        assert.is_nil(result.files[1].moved_from)
    end)

    it("shows a staged rename", function()
        git({ "mv", "a.txt", "b.txt" })
        write(repo, "b.txt", EDITED)
        git({ "add", "b.txt" })

        local result = lib.run_diff_staged("git")

        assert.are.equal(1, count(result))
        assert_edited_rename(result, "a.txt", "b.txt", true)
    end)

    it("does not pair a deletion with an unrelated addition", function()
        vim.fn.delete(repo .. "/a.txt")
        write(repo, "other.txt", { "something", "else", "entirely" })
        commit("delete and add")

        local result = lib.run_diff("HEAD", "git")
        local files = by_path(result)

        assert.are.equal(2, count(result))
        assert.are.equal("deleted", files["a.txt"].status)
        assert.are.equal("created", files["other.txt"].status)
        assert.is_nil(files["other.txt"].moved_from)
        assert.are.same(BODY, side(files["a.txt"], "left"))
    end)
end)

for _, colocate in ipairs({ true, false }) do
    local kind = colocate and "colocated" or "non-colocated"

    describe("jj renames (" .. kind .. ")", function()
        if not (lib_ok and has_difft and vim.fn.executable("jj") == 1) then
            it("is skipped without the native library, difft or jj", function() end)
            return
        end

        local repo, original_cwd, saved_env

        local function jj(args)
            local cmd = { "jj" }
            vim.list_extend(cmd, args)
            return run(cmd, repo)
        end

        local function commit(message)
            jj({ "commit", "-m", message })
        end

        local function move(from, to)
            vim.fn.mkdir(vim.fn.fnamemodify(repo .. "/" .. to, ":h"), "p")
            assert.are.equal(0, vim.fn.rename(repo .. "/" .. from, repo .. "/" .. to))
        end

        --- Line counts come from git, so only a colocated repo has them.
        local function assert_stats(file, additions, deletions)
            if colocate then
                assert.are.equal(additions, file.additions)
                assert.are.equal(deletions, file.deletions)
            end
        end

        before_each(function()
            saved_env = { JJ_USER = vim.env.JJ_USER, JJ_EMAIL = vim.env.JJ_EMAIL }
            vim.env.JJ_USER = "t"
            vim.env.JJ_EMAIL = "t@t"
            original_cwd = vim.fn.getcwd()
            repo = vim.fn.resolve(vim.fn.tempname())
            vim.fn.mkdir(repo, "p")
            jj(colocate and { "git", "init", "--colocate" } or { "git", "init" })
            write(repo, "a.txt", BODY)
            write(repo, "src/old.txt", BODY)
            write(repo, "keep.txt", { "keep" })
            commit("initial")
            vim.fn.chdir(repo)
        end)

        after_each(function()
            vim.fn.chdir(original_cwd)
            vim.fn.delete(repo, "rf")
            vim.env.JJ_USER = saved_env.JJ_USER
            vim.env.JJ_EMAIL = saved_env.JJ_EMAIL
        end)

        it("shows a pure rename in a single revision", function()
            move("a.txt", "b.txt")
            commit("rename")

            local result = lib.run_diff("@-", "jj")

            assert.are.equal(1, count(result))
            assert_pure_rename(result, "a.txt", "b.txt")
        end)

        it("shows an edited rename in a single revision", function()
            move("a.txt", "b.txt")
            write(repo, "b.txt", EDITED)
            commit("rename and edit")

            local result = lib.run_diff("@-", "jj")

            assert.are.equal(1, count(result))
            assert_edited_rename(result, "a.txt", "b.txt")
            assert_stats(by_path(result)["b.txt"], 1, 1)
        end)

        it("highlights only the edited line of a renamed file", function()
            move("a.txt", "b.txt")
            write(repo, "b.txt", EDITED)
            commit("rename and edit")

            local file = by_path(lib.run_diff("@-", "jj"))["b.txt"]
            local highlighted = {}
            for _, row in ipairs(file.rows) do
                if #row.left.highlights > 0 or #row.right.highlights > 0 then
                    table.insert(highlighted, row.right.content)
                end
            end

            assert.are.same({ "THREE" }, highlighted)
        end)

        it("detects the language from the new file name", function()
            write(repo, "lib.rs", { "fn a() {}", "fn b() {}" })
            commit("add rust file")
            move("lib.rs", "main.rs")
            write(repo, "main.rs", { "fn a() {}", "fn c() {}" })
            commit("rename rust file")

            local file = by_path(lib.run_diff("@-", "jj"))["main.rs"]

            assert.are.equal("Rust", file.language)
            assert.are.equal("lib.rs", file.moved_from)
            assert.are.same({ "fn a() {}", "fn b() {}" }, side(file, "left"))
            assert.are.equal(1, #file.hunk_starts)
        end)

        it("shows a rename into another directory", function()
            move("a.txt", "dir/moved.txt")
            write(repo, "dir/moved.txt", EDITED)
            commit("move")

            local result = lib.run_diff("@-", "jj")

            assert.are.equal(1, count(result))
            assert_edited_rename(result, "a.txt", "dir/moved.txt")
            assert_stats(by_path(result)["dir/moved.txt"], 1, 1)
        end)

        it("shows a rename within a directory", function()
            move("src/old.txt", "src/new.txt")
            write(repo, "src/new.txt", EDITED)
            commit("rename in dir")

            local result = lib.run_diff("@-", "jj")

            assert.are.equal(1, count(result))
            assert_edited_rename(result, "src/old.txt", "src/new.txt")
            assert_stats(by_path(result)["src/new.txt"], 1, 1)
        end)

        it("shows several renames in one revision", function()
            move("a.txt", "b.txt")
            move("src/old.txt", "src/new.txt")
            write(repo, "src/new.txt", EDITED)
            commit("two renames")

            local result = lib.run_diff("@-", "jj")

            assert.are.equal(2, count(result))
            assert_pure_rename(result, "a.txt", "b.txt")
            assert_edited_rename(result, "src/old.txt", "src/new.txt")
        end)

        it("shows a rename inside a range", function()
            local base = jj({ "log", "-r", "@-", "--no-graph", "-T", "commit_id" })
            write(repo, "keep.txt", { "kept" })
            commit("unrelated")
            move("a.txt", "b.txt")
            write(repo, "b.txt", EDITED)
            commit("rename")

            local result = lib.run_diff(base .. "..@-", "jj")

            assert.are.equal(2, count(result))
            assert_edited_rename(result, "a.txt", "b.txt")
            assert_stats(by_path(result)["b.txt"], 1, 1)
        end)

        it("shows a rename in the working copy", function()
            move("a.txt", "b.txt")
            write(repo, "b.txt", EDITED)

            local result = lib.run_diff_unstaged("jj")

            assert.are.equal(1, count(result))
            assert_edited_rename(result, "a.txt", "b.txt")
        end)

        it("shows a working-copy rename for the staged fallback", function()
            move("a.txt", "b.txt")

            local result = lib.run_diff_staged("jj")

            assert.are.equal(1, count(result))
            assert_pure_rename(result, "a.txt", "b.txt")
        end)

        it("does not pair a deletion with an unrelated addition", function()
            vim.fn.delete(repo .. "/a.txt")
            write(repo, "other.txt", { "something", "else", "entirely" })
            commit("delete and add")

            local result = lib.run_diff("@-", "jj")
            local files = by_path(result)

            assert.are.equal(2, count(result))
            assert.are.equal("deleted", files["a.txt"].status)
            assert.are.equal("created", files["other.txt"].status)
            assert.is_nil(files["other.txt"].moved_from)
            assert.are.same(BODY, side(files["a.txt"], "left"))
        end)

        it("keeps a copied file as an addition next to its source", function()
            vim.fn.writefile(BODY, repo .. "/copy.txt")
            write(repo, "a.txt", EDITED)
            commit("copy and edit source")

            local result = lib.run_diff("@-", "jj")
            local files = by_path(result)

            assert.are.equal(2, count(result))
            assert.are.equal("created", files["copy.txt"].status)
            assert.is_nil(files["copy.txt"].moved_from)
            assert.are.same(BODY, side(files["copy.txt"], "right"))
            assert.are.same(BODY, side(files["a.txt"], "left"))
            assert.are.same(EDITED, side(files["a.txt"], "right"))
        end)
    end)
end
