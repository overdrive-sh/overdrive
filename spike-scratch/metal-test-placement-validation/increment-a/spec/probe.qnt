module probe {
  type Act = | ev_init | ev_step
  var x: int
  var conf_step: Act
  action init = all { x' = 0, conf_step' = ev_init }
  action step = all { x' = x + 1, conf_step' = ev_step }
  run counts = init.then(step).then(step)
}
