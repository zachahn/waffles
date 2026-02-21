# waffles

run tasks in parallel

## example

```sh
/usr/bin/env waffles

echo 1
echo 2
echo 3
echo 4
echo 5
```

output:

```
echo 5 | 5
echo 4 | 4
echo 3 | 3
echo 1 | 1
echo 2 | 2
```
