-- A number with a line binding is assigned (D8 finding 46,
-- docs/TENANT_SCOPE.md §3.7): the API now moves a number between
-- `unassigned` and `assigned` as its bindings come and go; the numbers
-- bound before that still say `unassigned`. Other states are left alone.
UPDATE phone_numbers SET state = 'assigned', updated_at = NOW()
 WHERE state = 'unassigned'
   AND id IN (SELECT phone_number_id FROM line_bindings);
