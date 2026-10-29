from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="PageDirectoryEntryDataItem")



@_attrs_define
class PageDirectoryEntryDataItem:
    """ Someone, or something, in the tenant's directory.

        Attributes:
            kind (str): `user`, or `extension` for one that rings no user (a queue, a
                flow, a number).
            user_id (None | str | Unset): The user's id; unset for an extension.
            username (None | str | Unset):
            display_name (None | str | Unset):
            extension (None | str | Unset):
            email (None | str | Unset):
            department (None | str | Unset):
            job_title (None | str | Unset):
            presence (None | str | Unset): What others see of them (`docs/PRESENCE.md`): for a user,
                `available`, `away`, `busy`, `dnd`, `ringing`, `on_call` or
                `offline`; for an extension, its SIP presence (`open`, `closed`,
                `away`, `busy`, `on_the_phone`, `ringing`); unset when nothing is
                known.
            presence_label (None | str | Unset): A user's own status's label ("WFH").
            presence_note (None | str | Unset):
            presence_idle (bool | Unset): A user shown away because their apps say they are idle.
     """

    kind: str
    user_id: None | str | Unset = UNSET
    username: None | str | Unset = UNSET
    display_name: None | str | Unset = UNSET
    extension: None | str | Unset = UNSET
    email: None | str | Unset = UNSET
    department: None | str | Unset = UNSET
    job_title: None | str | Unset = UNSET
    presence: None | str | Unset = UNSET
    presence_label: None | str | Unset = UNSET
    presence_note: None | str | Unset = UNSET
    presence_idle: bool | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        kind = self.kind

        user_id: None | str | Unset
        if isinstance(self.user_id, Unset):
            user_id = UNSET
        else:
            user_id = self.user_id

        username: None | str | Unset
        if isinstance(self.username, Unset):
            username = UNSET
        else:
            username = self.username

        display_name: None | str | Unset
        if isinstance(self.display_name, Unset):
            display_name = UNSET
        else:
            display_name = self.display_name

        extension: None | str | Unset
        if isinstance(self.extension, Unset):
            extension = UNSET
        else:
            extension = self.extension

        email: None | str | Unset
        if isinstance(self.email, Unset):
            email = UNSET
        else:
            email = self.email

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

        presence: None | str | Unset
        if isinstance(self.presence, Unset):
            presence = UNSET
        else:
            presence = self.presence

        presence_label: None | str | Unset
        if isinstance(self.presence_label, Unset):
            presence_label = UNSET
        else:
            presence_label = self.presence_label

        presence_note: None | str | Unset
        if isinstance(self.presence_note, Unset):
            presence_note = UNSET
        else:
            presence_note = self.presence_note

        presence_idle = self.presence_idle


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "kind": kind,
        })
        if user_id is not UNSET:
            field_dict["user_id"] = user_id
        if username is not UNSET:
            field_dict["username"] = username
        if display_name is not UNSET:
            field_dict["display_name"] = display_name
        if extension is not UNSET:
            field_dict["extension"] = extension
        if email is not UNSET:
            field_dict["email"] = email
        if department is not UNSET:
            field_dict["department"] = department
        if job_title is not UNSET:
            field_dict["job_title"] = job_title
        if presence is not UNSET:
            field_dict["presence"] = presence
        if presence_label is not UNSET:
            field_dict["presence_label"] = presence_label
        if presence_note is not UNSET:
            field_dict["presence_note"] = presence_note
        if presence_idle is not UNSET:
            field_dict["presence_idle"] = presence_idle

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        kind = d.pop("kind")

        def _parse_user_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        user_id = _parse_user_id(d.pop("user_id", UNSET))


        def _parse_username(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        username = _parse_username(d.pop("username", UNSET))


        def _parse_display_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        display_name = _parse_display_name(d.pop("display_name", UNSET))


        def _parse_extension(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        extension = _parse_extension(d.pop("extension", UNSET))


        def _parse_email(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        email = _parse_email(d.pop("email", UNSET))


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


        def _parse_presence(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        presence = _parse_presence(d.pop("presence", UNSET))


        def _parse_presence_label(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        presence_label = _parse_presence_label(d.pop("presence_label", UNSET))


        def _parse_presence_note(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        presence_note = _parse_presence_note(d.pop("presence_note", UNSET))


        presence_idle = d.pop("presence_idle", UNSET)

        page_directory_entry_data_item = cls(
            kind=kind,
            user_id=user_id,
            username=username,
            display_name=display_name,
            extension=extension,
            email=email,
            department=department,
            job_title=job_title,
            presence=presence,
            presence_label=presence_label,
            presence_note=presence_note,
            presence_idle=presence_idle,
        )


        page_directory_entry_data_item.additional_properties = d
        return page_directory_entry_data_item

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
