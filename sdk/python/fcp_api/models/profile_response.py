from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="ProfileResponse")



@_attrs_define
class ProfileResponse:
    """ Profile response (limited fields for end users).

        Attributes:
            username (str):
            language (str):
            timezone (str):
            role (str):
            has_profile_picture (bool):
            directory_listed (bool): Listed in the tenant's directory for its users.
            may_change_directory_listing (bool): The tenant lets its users take themselves out of the directory, or
                put themselves back (`directory_listed` on `PUT /me/profile`).
            display_name (None | str | Unset): The name the user is shown by: their display name, else their first
                and last name.
            first_name (None | str | Unset):
            last_name (None | str | Unset):
            email (None | str | Unset):
            phone_number (None | str | Unset):
            extension (None | str | Unset):
            department (None | str | Unset):
            job_title (None | str | Unset):
            location (None | str | Unset):
     """

    username: str
    language: str
    timezone: str
    role: str
    has_profile_picture: bool
    directory_listed: bool
    may_change_directory_listing: bool
    display_name: None | str | Unset = UNSET
    first_name: None | str | Unset = UNSET
    last_name: None | str | Unset = UNSET
    email: None | str | Unset = UNSET
    phone_number: None | str | Unset = UNSET
    extension: None | str | Unset = UNSET
    department: None | str | Unset = UNSET
    job_title: None | str | Unset = UNSET
    location: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        username = self.username

        language = self.language

        timezone = self.timezone

        role = self.role

        has_profile_picture = self.has_profile_picture

        directory_listed = self.directory_listed

        may_change_directory_listing = self.may_change_directory_listing

        display_name: None | str | Unset
        if isinstance(self.display_name, Unset):
            display_name = UNSET
        else:
            display_name = self.display_name

        first_name: None | str | Unset
        if isinstance(self.first_name, Unset):
            first_name = UNSET
        else:
            first_name = self.first_name

        last_name: None | str | Unset
        if isinstance(self.last_name, Unset):
            last_name = UNSET
        else:
            last_name = self.last_name

        email: None | str | Unset
        if isinstance(self.email, Unset):
            email = UNSET
        else:
            email = self.email

        phone_number: None | str | Unset
        if isinstance(self.phone_number, Unset):
            phone_number = UNSET
        else:
            phone_number = self.phone_number

        extension: None | str | Unset
        if isinstance(self.extension, Unset):
            extension = UNSET
        else:
            extension = self.extension

        department: None | str | Unset
        if isinstance(self.department, Unset):
            department = UNSET
        else:
            department = self.department

        job_title: None | str | Unset
        if isinstance(self.job_title, Unset):
            job_title = UNSET
        else:
            job_title = self.job_title

        location: None | str | Unset
        if isinstance(self.location, Unset):
            location = UNSET
        else:
            location = self.location


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "username": username,
            "language": language,
            "timezone": timezone,
            "role": role,
            "has_profile_picture": has_profile_picture,
            "directory_listed": directory_listed,
            "may_change_directory_listing": may_change_directory_listing,
        })
        if display_name is not UNSET:
            field_dict["display_name"] = display_name
        if first_name is not UNSET:
            field_dict["first_name"] = first_name
        if last_name is not UNSET:
            field_dict["last_name"] = last_name
        if email is not UNSET:
            field_dict["email"] = email
        if phone_number is not UNSET:
            field_dict["phone_number"] = phone_number
        if extension is not UNSET:
            field_dict["extension"] = extension
        if department is not UNSET:
            field_dict["department"] = department
        if job_title is not UNSET:
            field_dict["job_title"] = job_title
        if location is not UNSET:
            field_dict["location"] = location

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        username = d.pop("username")

        language = d.pop("language")

        timezone = d.pop("timezone")

        role = d.pop("role")

        has_profile_picture = d.pop("has_profile_picture")

        directory_listed = d.pop("directory_listed")

        may_change_directory_listing = d.pop("may_change_directory_listing")

        def _parse_display_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        display_name = _parse_display_name(d.pop("display_name", UNSET))


        def _parse_first_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        first_name = _parse_first_name(d.pop("first_name", UNSET))


        def _parse_last_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        last_name = _parse_last_name(d.pop("last_name", UNSET))


        def _parse_email(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        email = _parse_email(d.pop("email", UNSET))


        def _parse_phone_number(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        phone_number = _parse_phone_number(d.pop("phone_number", UNSET))


        def _parse_extension(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        extension = _parse_extension(d.pop("extension", UNSET))


        def _parse_department(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        department = _parse_department(d.pop("department", UNSET))


        def _parse_job_title(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        job_title = _parse_job_title(d.pop("job_title", UNSET))


        def _parse_location(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        location = _parse_location(d.pop("location", UNSET))


        profile_response = cls(
            username=username,
            language=language,
            timezone=timezone,
            role=role,
            has_profile_picture=has_profile_picture,
            directory_listed=directory_listed,
            may_change_directory_listing=may_change_directory_listing,
            display_name=display_name,
            first_name=first_name,
            last_name=last_name,
            email=email,
            phone_number=phone_number,
            extension=extension,
            department=department,
            job_title=job_title,
            location=location,
        )


        profile_response.additional_properties = d
        return profile_response

    @property
    def additional_keys(self) -> list[str]:
        return list(self.additional_properties.keys())

    def __getitem__(self, key: str) -> Any:
        return self.additional_properties[key]

    def __setitem__(self, key: str, value: Any) -> None:
        self.additional_properties[key] = value

    def __delitem__(self, key: str) -> None:
        del self.additional_properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.additional_properties
