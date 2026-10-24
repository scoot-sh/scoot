-- The "ginger-night" Neovim colorscheme: Ginger night, ember ring. The look's own
-- palette, matching its apps/helix.toml (same hues, same roles). MIT.
-- The look's apps/nvim.lua static leaf.

vim.cmd("hi clear")
if vim.fn.exists("syntax_on") == 1 then
  vim.cmd("syntax reset")
end
vim.g.colors_name = "scoot-look"

local c = {
  ink = "#f5ead6",
  slate = "#c9b8a3",
  mist = "#8f7a63",
  paper = "#0e0e0e",
  blue = "#e57f29",
  red = "#e05a4e",
  green = "#a9b98f",
  yellow = "#ffb14b",
  purple = "#c08a7a",
  teal = "#8aa8a0",
  surface = "#161210",
  selection = "#4a2c17",
  cursorline = "#241a12",
  guide = "#33261a",
}

local function hi(group, opts)
  vim.api.nvim_set_hl(0, group, opts)
end

-- Editor chrome: the background stays transparent (like the Helix leaf's
-- `"ui.background" = {}`), so a translucent terminal shows through.
hi("Normal", { fg = c.ink })
hi("NormalFloat", { fg = c.ink, bg = c.surface })
hi("FloatBorder", { fg = c.mist, bg = c.surface })
hi("CursorLine", { bg = c.cursorline })
hi("CursorColumn", { bg = c.cursorline })
hi("CursorLineNr", { fg = c.blue, bold = true })
hi("LineNr", { fg = c.mist })
hi("SignColumn", {})
hi("Visual", { bg = c.selection })
hi("Search", { fg = c.paper, bg = c.blue })
hi("IncSearch", { fg = c.paper, bg = c.blue })
hi("CurSearch", { fg = c.paper, bg = c.blue })
hi("Pmenu", { fg = c.ink, bg = c.surface })
hi("PmenuSel", { fg = c.paper, bg = c.blue })
hi("PmenuThumb", { bg = c.mist })
hi("StatusLine", { fg = c.ink, bg = c.surface })
hi("StatusLineNC", { fg = c.mist, bg = c.surface })
hi("TabLine", { fg = c.mist, bg = c.surface })
hi("TabLineSel", { fg = c.ink, bold = true })
hi("WinSeparator", { fg = c.mist })
hi("Folded", { fg = c.mist, bg = c.surface })
hi("FoldColumn", { fg = c.mist })
hi("ColorColumn", { bg = c.surface })
hi("NonText", { fg = c.guide })
hi("Whitespace", { fg = c.guide })
hi("SpecialKey", { fg = c.guide })
hi("Directory", { fg = c.blue })
hi("Title", { fg = c.blue, bold = true })
hi("Question", { fg = c.green })
hi("MoreMsg", { fg = c.green })
hi("ModeMsg", { bold = true })
hi("ErrorMsg", { fg = c.red })
hi("WarningMsg", { fg = c.yellow })
hi("MatchParen", { fg = c.paper, bg = c.blue, bold = true })
hi("WildMenu", { fg = c.paper, bg = c.blue })

-- Syntax: the Helix roles, one to one.
hi("Comment", { fg = c.mist, italic = true })
hi("Constant", { fg = c.yellow })
hi("String", { fg = c.green })
hi("Character", { fg = c.green })
hi("Number", { fg = c.yellow })
hi("Boolean", { fg = c.yellow })
hi("Float", { fg = c.yellow })
hi("Identifier", { fg = c.ink })
hi("Function", { fg = c.blue })
hi("Statement", { fg = c.red, bold = true })
hi("Conditional", { fg = c.red })
hi("Repeat", { fg = c.red })
hi("Label", { fg = c.purple })
hi("Operator", { fg = c.red })
hi("Keyword", { fg = c.red })
hi("Exception", { fg = c.red })
hi("PreProc", { fg = c.teal })
hi("Include", { fg = c.teal })
hi("Define", { fg = c.teal })
hi("Macro", { fg = c.teal })
hi("Type", { fg = c.purple })
hi("StorageClass", { fg = c.purple })
hi("Structure", { fg = c.purple })
hi("Typedef", { fg = c.purple })
hi("Special", { fg = c.red })
hi("SpecialChar", { fg = c.teal })
hi("Delimiter", { fg = c.slate })
hi("Underlined", { underline = true })
hi("Error", { fg = c.red })
hi("Todo", { fg = c.yellow, bold = true })

-- Diffs and diagnostics.
hi("DiffAdd", { fg = c.green })
hi("DiffChange", { fg = c.yellow })
hi("DiffDelete", { fg = c.red })
hi("DiffText", { fg = c.blue, bold = true })
hi("DiagnosticError", { fg = c.red })
hi("DiagnosticWarn", { fg = c.yellow })
hi("DiagnosticInfo", { fg = c.blue })
hi("DiagnosticHint", { fg = c.teal })
hi("DiagnosticUnderlineError", { undercurl = true, sp = c.red })
hi("DiagnosticUnderlineWarn", { undercurl = true, sp = c.yellow })
hi("DiagnosticUnderlineInfo", { undercurl = true, sp = c.blue })
hi("DiagnosticUnderlineHint", { undercurl = true, sp = c.teal })
hi("SpellBad", { undercurl = true, sp = c.red })
hi("SpellCap", { undercurl = true, sp = c.blue })
hi("SpellRare", { undercurl = true, sp = c.purple })
hi("SpellLocal", { undercurl = true, sp = c.teal })

-- Live reload without a signal: this file watches itself and re-sources
-- on change, recoloring running nvim within about a second. `scoot look
-- apply` repoints the look link behind this file, `scoot look watch`
-- rewrites the render in place: both arrive here as file events. The
-- guard keeps a re-source from stacking watchers (sourcing this file
-- twice arms one set of handles, never two; a plain Lua global, since
-- vim.g would convert the table through viml and drop the handles).
if _G.scoot_look_nvim_watching == nil and vim.uv ~= nil and vim.uv.new_fs_event ~= nil then
  _G.scoot_look_nvim_watching = { handles = {}, pending = false }
  local watcher = _G.scoot_look_nvim_watching
  local src = debug.getinfo(1, "S").source:sub(2)

  local function reselect()
    if watcher.pending then
      return
    end
    watcher.pending = true
    vim.schedule(function()
      watcher.pending = false
      -- Through the wired colorscheme name when it resolves (the file
      -- lives on the runtimepath as `scoot-look`), else straight back
      -- through the link at whatever it names now (sourced by path,
      -- never wired: the name is unknown to `:colorscheme`).
      local ok = pcall(vim.cmd, "colorscheme scoot-look")
      if not ok then
        local ok_real, current = pcall(vim.uv.fs_realpath, src)
        if ok_real and type(current) == "string" then
          pcall(vim.cmd, "source " .. vim.fn.fnameescape(current))
        end
      end
    end)
  end

  local function arm(path)
    if path == nil or path == "" then
      return
    end
    -- A replace or a repoint orphans the previous handle (the swap moved
    -- what the path named), so stop it first: one path holds one handle.
    local old = watcher.handles[path]
    if old ~= nil then
      pcall(function()
        old:stop()
      end)
      watcher.handles[path] = nil
    end
    local handle = vim.uv.new_fs_event()
    if handle == nil then
      return
    end
    local ok = handle:start(path, {}, function(err)
      if err ~= nil then
        return
      end
      -- Re-arm onto whatever the path names now, then re-source once per
      -- burst (a save fires several events; `pending` coalesces them).
      vim.schedule(function()
        arm(path)
      end)
      reselect()
    end)
    if ok == nil or ok == false then
      return
    end
    watcher.handles[path] = handle
  end

  -- The file itself (in-place edits) and its directory (a swap replaces
  -- the file, a rebuild replaces the link: the directory entry moves).
  arm(src)
  arm(vim.fn.fnamemodify(src, ":h"))
  -- The state directory behind the look link: an apply repoints the link
  -- itself, which fires in the state dir and nowhere under the old
  -- render. The file cannot derive it from its own path (sourcing
  -- resolves the link away), so it mirrors the tool's own resolution:
  -- `$SCOOT_LOOK_STATE`, then `$XDG_STATE_HOME/scoot`, then
  -- `~/.local/state/scoot`. A missing dir simply fails to arm.
  local states = {}
  if vim.env.SCOOT_LOOK_STATE ~= nil and vim.env.SCOOT_LOOK_STATE ~= "" then
    table.insert(states, vim.env.SCOOT_LOOK_STATE)
  end
  if vim.env.XDG_STATE_HOME ~= nil and vim.env.XDG_STATE_HOME ~= "" then
    table.insert(states, vim.env.XDG_STATE_HOME .. "/scoot")
  else
    local home = vim.uv.os_homedir()
    if home ~= nil and home ~= "" then
      table.insert(states, home .. "/.local/state/scoot")
    end
  end
  for _, state in ipairs(states) do
    arm(state)
  end
  -- The render cache behind the slot: `scoot look watch` swaps the slot
  -- directory itself (rename aside, rename in), which orphans the file
  -- and directory watches above without firing on every platform. The
  -- cache directory is stable, so its entry events announce the swap.
  -- Only when the resolved file sits in an `apps/` dir (the render
  -- shape): a store file has no slot to swap.
  local ok_cache, resolved = pcall(vim.uv.fs_realpath, src)
  if ok_cache and type(resolved) == "string" then
    local slot = resolved:match("^(.*)/apps/nvim%.lua$")
    if slot ~= nil then
      local cache = vim.fn.fnamemodify(slot, ":h")
      if cache ~= nil and cache ~= "" then
        arm(cache)
      end
    end
  end
end
