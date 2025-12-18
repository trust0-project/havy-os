# Bootstrapping Release Please

Release Please requires an initial git tag to exist that matches the version in `.release-please-manifest.json`.

## Initial Setup

Since you're migrating to Release Please with an existing version (0.1.12), you need to create an initial tag:

```bash
# Create a tag for the current version
git tag v0.1.12

# Push the tag to GitHub
git push origin v0.1.12
```

After creating this initial tag, Release Please will work correctly and will:
1. Track changes based on conventional commits
2. Create release PRs when changes are detected
3. Update versions and generate changelogs automatically

## Next Steps

Once the initial tag is pushed:
1. Commit any new changes using conventional commits (e.g., `feat:`, `fix:`)
2. Push to the `main` branch
3. Release Please will automatically create a release PR
4. Merge the PR to trigger the release and asset build

## Alternative: Start Fresh

If you prefer to start from scratch, you can instead:

```bash
# Update the manifest to start from 0.0.0
# Edit .release-please-manifest.json and set version to "0.0.0"
# Edit Cargo.toml files to use version 0.0.0
# Commit these changes
# Release Please will create its first release from 0.0.0
```

However, starting from 0.1.12 is recommended since that's your current version.









