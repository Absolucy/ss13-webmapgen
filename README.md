a doohickey to generate SS13 webmaps, intended to replace the `generateMaps` script in https://github.com/Monkestation/ss13-webmap

it's much faster - it only parses each dme once, generates map files in parallel, and outputs both optimized pngs and webp files.

this uses the exact same config format (see [config.example.json](config.example.json)) and outputs the same file structure - should be drop-in compatible, pretty much.

speed example - 3 maps from OculisStation
```
MetaStation: dim_x=255, dim_y=255, dim_z=1
Void Raptor: dim_x=255, dim_y=255, dim_z=1
SerenityStation: dim_x=255, dim_y=255, dim_z=3
NebulaStation: dim_x=255, dim_y=255, dim_z=2
done :)
  config load:  322.30µs
  env parse:    7.09s
  render:       1.24s
  encode:       38.17s
```
note: encode time is almost **entirely** spent on png optimization - disabling pngs entirely and only using webps can result in much faster generation times
