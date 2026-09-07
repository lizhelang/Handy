-- 实际 Lua filter 执行证据；不是只检查插件符号存在。
function static_probe_filter(input, env)
  for candidate in input:iter() do
    candidate.comment = "static-lua-executed"
    yield(candidate)
  end
end
