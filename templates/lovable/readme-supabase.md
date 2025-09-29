# ${{ lovable_project_name }}

## Project info

This project was imported from [Lovable](https://lovable.dev) using [lttle.cloud](https://lttle.cloud) platform. You can view the original project [here](https://lovable.dev/projects/${{ lovable_project_id }})


## Running the project locally

- Install the dependencies with `npm install`
- Run the project with `npm run dev`
- Open the project in the browser at `http://localhost:8080`

## Deploying the project

- Run `lttle deploy` to deploy the project
- Use `lttle machine get ${{ lovable_project_name }}` to find more about the running machine (eg: status, configuration, etc.)
- Use `lttle app get ${{ lovable_project_name }}` to find more about the deployed app (eg: port, assigned URLs, etc.)

If you're not sure what commands to use, what a command does or what arguments a command accept, you can add the `--help` flag to the command to get more information.

## Using Supabase

The Supabase client is already installed and configured to use the [this project](https://supabase.com/dashboard/project/${{ supabase_project_id }}) (the same one used in Lovable).

Make sure you are authenticated by running `npm exec supabase login`.

If you make modifications to the database schema, you can run `npm run update-supabase` to regenerate the client code.

## What technologies are used for this project?

This project is built with:

- Vite
- TypeScript
- React
- shadcn-ui
- Tailwind CSS
- Supabase
