--- Binary management: loading, downloading, and building.
local M = {}

local GITHUB_REPO = "clabby/difftastic.nvim"

--- Platform detection
--- @return string|nil platform Triple like "aarch64-apple-darwin"
--- @return string|nil ext Library extension like ".dylib"
local function get_platform()
    local os_name = jit.os:lower()
    local arch = jit.arch:lower()

    if os_name == "osx" then
        local triple = arch == "arm64" and "aarch64-apple-darwin" or "x86_64-apple-darwin"
        return triple, ".dylib"
    elseif os_name == "linux" then
        local triple = arch == "arm64" and "aarch64-unknown-linux-gnu" or "x86_64-unknown-linux-gnu"
        return triple, ".so"
    elseif os_name == "windows" then
        return "x86_64-pc-windows-msvc", ".dll"
    end

    return nil, nil
end

--- Get the library filename for a given extension.
--- @param ext string|nil Library extension
--- @return string
local function get_lib_name(ext)
    if ext == ".dylib" then
        return "libdifftastic_nvim.dylib"
    elseif ext == ".dll" then
        return "difftastic_nvim.dll"
    else
        return "libdifftastic_nvim.so"
    end
end

--- Get paths used for library storage.
--- @return table { plugin_root: string, data_dir: string, release_dir: string }
local function get_paths()
    local source = debug.getinfo(1, "S").source:sub(2)
    local plugin_root = vim.fn.fnamemodify(source, ":h:h:h")
    local data_dir = vim.fn.stdpath("data") .. "/difftastic-nvim"

    return {
        plugin_root = plugin_root,
        data_dir = data_dir,
        release_dir = plugin_root .. "/target/release",
    }
end

--- Check if library exists in any of the expected locations.
--- @param paths table Path configuration
--- @param ext string|nil Library extension
--- @return boolean
local function lib_exists(paths, ext)
    local lib_name = get_lib_name(ext)
    return vim.uv.fs_stat(paths.data_dir .. "/" .. lib_name) ~= nil
        or vim.uv.fs_stat(paths.release_dir .. "/" .. lib_name) ~= nil
end

--- Try to load the library from a directory.
--- @param dir string Directory to load from
--- @param ext string|nil Library extension
--- @return table|nil Loaded library or nil
local function try_load_lib(dir, ext)
    local lib_name = get_lib_name(ext)
    local lib_path = dir .. "/" .. lib_name

    if not vim.uv.fs_stat(lib_path) then
        return nil
    end

    if ext == ".dll" then
        package.cpath = dir .. "/?.dll;" .. package.cpath
    else
        -- macOS/Linux: cargo produces lib*.dylib/lib*.so but Lua wants *.so
        local so_path = dir .. "/difftastic_nvim.so"
        if not vim.uv.fs_stat(so_path) then
            local ok, err = pcall(vim.uv.fs_symlink, lib_path, so_path)
            if not ok then
                vim.notify("difftastic-nvim: Failed to create symlink: " .. tostring(err), vim.log.levels.WARN)
            end
        end
        package.cpath = dir .. "/?.so;" .. package.cpath
    end

    local ok, lib = pcall(require, "difftastic_nvim")
    return ok and lib or nil
end

--- Version file management
local function get_version_file(paths)
    return paths.data_dir .. "/.version"
end

local function read_version(paths)
    local f = io.open(get_version_file(paths), "r")
    if not f then
        return nil
    end
    local version = f:read("*l")
    f:close()
    return version
end

local function write_version(paths, version)
    local f = io.open(get_version_file(paths), "w")
    if f then
        f:write(version)
        f:close()
    end
end

--- State machine for build/download status
--- @type "ready"|"building"|"downloading"|"failed"
M.state = "ready"

--- Cached library reference
local cached_lib = nil

--- Check for available updates (async, background).
--- @param paths table Path configuration
local function check_for_updates(paths)
    local local_version = read_version(paths)
    if not local_version then
        return
    end

    local url = "https://api.github.com/repos/" .. GITHUB_REPO .. "/releases/latest"
    vim.system({ "curl", "-sL", url }, { text = true }, function(result)
        if result.code ~= 0 then
            return
        end

        local ok, release = pcall(vim.json.decode, result.stdout)
        if not ok or not release or not release.tag_name then
            return
        end

        if release.tag_name ~= local_version then
            vim.schedule(function()
                vim.notify(
                    string.format(
                        "difftastic-nvim: Update available (%s -> %s). Run :DifftUpdate",
                        local_version,
                        release.tag_name
                    ),
                    vim.log.levels.INFO
                )
            end)
        end
    end)
end

--- Download binary from GitHub releases (async).
--- @param paths table Path configuration
--- @param platform string Platform triple
--- @param ext string|nil Library extension
--- @param on_complete function|nil Callback with success boolean
local function download_binary(paths, platform, ext, on_complete)
    M.state = "downloading"
    vim.notify("difftastic-nvim: Downloading binary...", vim.log.levels.INFO)
    vim.fn.mkdir(paths.data_dir, "p")

    local url = "https://api.github.com/repos/" .. GITHUB_REPO .. "/releases/latest"
    vim.system({ "curl", "-sL", url }, { text = true }, function(result)
        if result.code ~= 0 then
            vim.schedule(function()
                M.state = "failed"
                vim.notify("difftastic-nvim: Failed to fetch release info", vim.log.levels.ERROR)
                if on_complete then
                    on_complete(false)
                end
            end)
            return
        end

        local ok, release = pcall(vim.json.decode, result.stdout)
        if not ok or not release or not release.assets then
            vim.schedule(function()
                M.state = "failed"
                vim.notify("difftastic-nvim: No release found", vim.log.levels.ERROR)
                if on_complete then
                    on_complete(false)
                end
            end)
            return
        end

        -- Find matching asset
        local asset_name = platform .. ext
        local download_url = nil
        for _, asset in ipairs(release.assets) do
            if asset.name == asset_name then
                download_url = asset.browser_download_url
                break
            end
        end

        if not download_url then
            vim.schedule(function()
                M.state = "failed"
                vim.notify("difftastic-nvim: No binary for " .. platform, vim.log.levels.ERROR)
                if on_complete then
                    on_complete(false)
                end
            end)
            return
        end

        local dest = paths.data_dir .. "/" .. get_lib_name(ext)
        vim.system({ "curl", "-sL", "-o", dest, download_url }, {}, function(dl_result)
            vim.schedule(function()
                if dl_result.code == 0 then
                    write_version(paths, release.tag_name)
                    M.state = "ready"
                    vim.notify("difftastic-nvim: Downloaded " .. release.tag_name, vim.log.levels.INFO)
                    if on_complete then
                        on_complete(true)
                    end
                else
                    M.state = "failed"
                    vim.notify("difftastic-nvim: Download failed", vim.log.levels.ERROR)
                    if on_complete then
                        on_complete(false)
                    end
                end
            end)
        end)
    end)
end

--- Build library from source (async).
--- @param paths table Path configuration
local function build_from_source(paths)
    M.state = "building"
    vim.notify("difftastic-nvim: Building from source...", vim.log.levels.INFO)

    vim.system({ "cargo", "build", "--release" }, { cwd = paths.plugin_root, text = true }, function(result)
        vim.schedule(function()
            if result.code == 0 then
                M.state = "ready"
                vim.notify("difftastic-nvim: Build complete", vim.log.levels.INFO)
            else
                M.state = "failed"
                vim.notify(
                    "difftastic-nvim: Build failed: " .. (result.stderr or "unknown error"),
                    vim.log.levels.ERROR
                )
            end
        end)
    end)
end

--- Ensure library exists, triggering download or build if needed.
--- Called at setup time.
--- @param download_enabled boolean Whether auto-download is enabled
function M.ensure_exists(download_enabled)
    local platform, ext = get_platform()
    local paths = get_paths()

    if lib_exists(paths, ext) then
        M.state = "ready"
        if download_enabled then
            check_for_updates(paths)
        end
        return
    end

    -- Try auto-download
    if download_enabled and platform then
        download_binary(paths, platform, ext)
        return
    end

    -- Try building from source
    local cargo_toml = paths.plugin_root .. "/Cargo.toml"
    if vim.uv.fs_stat(cargo_toml) then
        build_from_source(paths)
        return
    end

    -- No way to get the library
    M.state = "failed"
    vim.notify(
        string.format(
            "difftastic-nvim: Could not find %s.\nSet download = true, or install Rust toolchain.",
            get_lib_name(ext or ".so")
        ),
        vim.log.levels.ERROR
    )
end

--- Get the loaded library, erroring if not available.
--- @return table The loaded Rust library
function M.get()
    if cached_lib then
        return cached_lib
    end

    if M.state == "building" then
        error("difftastic-nvim: Still building, please wait...")
    elseif M.state == "downloading" then
        error("difftastic-nvim: Still downloading, please wait...")
    elseif M.state == "failed" then
        error("difftastic-nvim: Library not available. Check :messages for details.")
    end

    local _, ext = get_platform()
    local paths = get_paths()

    -- Prefer local development build in target/release over downloaded binary.
    cached_lib = try_load_lib(paths.release_dir, ext) or try_load_lib(paths.data_dir, ext)

    if cached_lib then
        return cached_lib
    end

    error("difftastic-nvim: Library not found.")
end

--- Update binary to latest release.
function M.update()
    local platform, ext = get_platform()
    if not platform then
        vim.notify("difftastic-nvim: Unsupported platform", vim.log.levels.ERROR)
        return
    end

    local paths = get_paths()

    -- Remove old files
    local lib_path = paths.data_dir .. "/" .. get_lib_name(ext)
    local so_path = paths.data_dir .. "/difftastic_nvim.so"
    vim.uv.fs_unlink(lib_path)
    vim.uv.fs_unlink(so_path)

    -- Clear cache
    package.loaded["difftastic_nvim"] = nil
    cached_lib = nil

    download_binary(paths, platform, ext)
end

return M
