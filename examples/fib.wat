(module
  (func $fib (param $n i64) (result i64)
    local.get $n
    i64.const 2
    i64.lt_s
    if (result i64)
      local.get $n
    else
      local.get $n
      i64.const 1
      i64.sub
      call $fib
      local.get $n
      i64.const 2
      i64.sub
      call $fib
      i64.add
    end)
  (func (export "run") (param i64) (result i64)
    local.get 0
    call $fib))
