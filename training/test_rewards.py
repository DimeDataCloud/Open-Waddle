"""Checks the GRPO click reward without a GPU:  python training/test_rewards.py"""

from grpo_grounding import click_reward

box = [100, 100, 200, 140]
centre = click_reward('{"x": 150, "y": 120}', box)
edge = click_reward('{"x": 199, "y": 139}', box)
outside = click_reward('{"x": 260, "y": 120}', box)
far = click_reward('{"x": 900, "y": 900}', box)
assert centre > edge > outside > far > 0, (centre, edge, outside, far)
assert abs(centre - 1.0) < 1e-9
assert click_reward("I can't see the screen", box) == 0.0
assert click_reward('click({"x": 150.5, "y": 120})', box) > 0.9
print("ok", round(centre, 3), round(edge, 3), round(outside, 3), round(far, 3))
