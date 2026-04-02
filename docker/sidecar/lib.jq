def dmerge($a; $b):
  if   ($a|type)=="object" and ($b|type)=="object" then
    reduce ($a + $b | keys_unsorted[]) as $k ({}; .[$k] = dmerge($a[$k]; $b[$k]))
  elif ($a|type)=="array"  and ($b|type)=="array"  then
    ($a + $b | unique)
  else
    $b // $a // false                       # prefer right side, fall back to left if null
  end;
