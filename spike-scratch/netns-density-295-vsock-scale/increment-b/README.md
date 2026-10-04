# Attempt b

The canonical launcher acquired its exclusive lease but source synchronization failed because attempt a created a root-owned Python import cache. No build or devices ran. A separately retained, canonical-lease cleanup removed only the witnessed exact owned cache file. Attempt c disables Python import cache writes before loading the helper.
