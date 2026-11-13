-- luacheck: globals vim
-- The selected note is an acwrite buffer. Native :write must go through the
-- host's conditional filesystem adapter, never Neovim's direct file writer.
local channel, path, lines = ...
local previous = vim.g.papertrail_buffer
if previous and vim.api.nvim_buf_is_valid(previous) then
  -- The host saved the old note before requesting this transition.
  vim.api.nvim_buf_delete(previous, { force = true })
end
local buffer = vim.api.nvim_create_buf(true, false)
vim.g.papertrail_buffer = buffer
vim.bo[buffer].buftype = 'acwrite'
vim.bo[buffer].swapfile = false
if path ~= '' then vim.api.nvim_buf_set_name(buffer, path) end
local managed_name = vim.api.nvim_buf_get_name(buffer)
vim.api.nvim_buf_set_lines(buffer, 0, -1, false, lines)
vim.bo[buffer].modified = false
vim.api.nvim_create_autocmd('BufWriteCmd', {
  buffer = buffer,
  callback = function(event)
    local name = vim.api.nvim_buf_get_name(buffer)
    if name ~= managed_name then
      error('Use F2 then rename/saveas to change the Paper Trail note filename')
    end
    if event.match ~= '' and vim.fn.fnamemodify(event.match, ':p') ~= vim.fn.fnamemodify(name, ':p') then
      error('Use F2 then :saveas PATH to write a different file safely')
    end
    vim.api.nvim_exec_autocmds('BufWritePre', { buffer = buffer, modeline = false })
    local tick = vim.api.nvim_buf_get_changedtick(buffer)
    vim.rpcrequest(channel, 'papertrail_write', buffer, tick,
      vim.api.nvim_buf_get_lines(buffer, 0, -1, false))
    if vim.api.nvim_buf_get_changedtick(buffer) == tick then
      vim.bo[buffer].modified = false
    end
    vim.api.nvim_exec_autocmds('BufWritePost', { buffer = buffer, modeline = false })
  end,
})
vim.api.nvim_set_current_buf(buffer)
if path ~= '' and vim.filetype and vim.filetype.match then
  local filetype = vim.filetype.match({ filename = path, buf = buffer })
  if filetype then vim.bo[buffer].filetype = filetype end
end
return buffer
