from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.media_preferences import MediaPreferences





T = TypeVar("T", bound="UpdateUserRequest")



@_attrs_define
class UpdateUserRequest:
    """ Update user request.

        Attributes:
            username (None | str | Unset): A new username: the user is renamed (`docs/USERS.md` §6). Their
                phones keep registering under their SIP name, which stays.
            display_name (None | str | Unset):
            enabled (bool | None | Unset):
            role (None | str | Unset):
            department_id (None | str | Unset):
            email (None | str | Unset):
            phone_number (None | str | Unset):
            mobile_number (None | str | Unset):
            pin (None | str | Unset):
            extension (None | str | Unset):
            department (None | str | Unset):
            job_title (None | str | Unset):
            location (None | str | Unset):
            location_id (None | str | Unset):
            language (None | str | Unset):
            timezone (None | str | Unset):
            account_status (None | str | Unset):
            emergency_contact_name (None | str | Unset):
            emergency_contact_phone (None | str | Unset):
            media_preferences (MediaPreferences | None | Unset):
     """

    username: None | str | Unset = UNSET
    display_name: None | str | Unset = UNSET
    enabled: bool | None | Unset = UNSET
    role: None | str | Unset = UNSET
    department_id: None | str | Unset = UNSET
    email: None | str | Unset = UNSET
    phone_number: None | str | Unset = UNSET
    mobile_number: None | str | Unset = UNSET
    pin: None | str | Unset = UNSET
    extension: None | str | Unset = UNSET
    department: None | str | Unset = UNSET
    job_title: None | str | Unset = UNSET
    location: None | str | Unset = UNSET
    location_id: None | str | Unset = UNSET
    language: None | str | Unset = UNSET
    timezone: None | str | Unset = UNSET
    account_status: None | str | Unset = UNSET
    emergency_contact_name: None | str | Unset = UNSET
    emergency_contact_phone: None | str | Unset = UNSET
    media_preferences: MediaPreferences | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.media_preferences import MediaPreferences # noqa: PLC0415
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

        enabled: bool | None | Unset
        if isinstance(self.enabled, Unset):
            enabled = UNSET
        else:
            enabled = self.enabled

        role: None | str | Unset
        if isinstance(self.role, Unset):
            role = UNSET
        else:
            role = self.role

        department_id: None | str | Unset
        if isinstance(self.department_id, Unset):
            department_id = UNSET
        else:
            department_id = self.department_id

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

        mobile_number: None | str | Unset
        if isinstance(self.mobile_number, Unset):
            mobile_number = UNSET
        else:
            mobile_number = self.mobile_number

        pin: None | str | Unset
        if isinstance(self.pin, Unset):
            pin = UNSET
        else:
            pin = self.pin

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

        location_id: None | str | Unset
        if isinstance(self.location_id, Unset):
            location_id = UNSET
        else:
            location_id = self.location_id

        language: None | str | Unset
        if isinstance(self.language, Unset):
            language = UNSET
        else:
            language = self.language

        timezone: None | str | Unset
        if isinstance(self.timezone, Unset):
            timezone = UNSET
        else:
            timezone = self.timezone

        account_status: None | str | Unset
        if isinstance(self.account_status, Unset):
            account_status = UNSET
        else:
            account_status = self.account_status

        emergency_contact_name: None | str | Unset
        if isinstance(self.emergency_contact_name, Unset):
            emergency_contact_name = UNSET
        else:
            emergency_contact_name = self.emergency_contact_name

        emergency_contact_phone: None | str | Unset
        if isinstance(self.emergency_contact_phone, Unset):
            emergency_contact_phone = UNSET
        else:
            emergency_contact_phone = self.emergency_contact_phone

        media_preferences: dict[str, Any] | None | Unset
        if isinstance(self.media_preferences, Unset):
            media_preferences = UNSET
        elif isinstance(self.media_preferences, MediaPreferences):
            media_preferences = self.media_preferences.to_dict()
        else:
            media_preferences = self.media_preferences


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
        })
        if username is not UNSET:
            field_dict["username"] = username
        if display_name is not UNSET:
            field_dict["display_name"] = display_name
        if enabled is not UNSET:
            field_dict["enabled"] = enabled
        if role is not UNSET:
            field_dict["role"] = role
        if department_id is not UNSET:
            field_dict["department_id"] = department_id
        if email is not UNSET:
            field_dict["email"] = email
        if phone_number is not UNSET:
            field_dict["phone_number"] = phone_number
        if mobile_number is not UNSET:
            field_dict["mobile_number"] = mobile_number
        if pin is not UNSET:
            field_dict["pin"] = pin
        if extension is not UNSET:
            field_dict["extension"] = extension
        if department is not UNSET:
            field_dict["department"] = department
        if job_title is not UNSET:
            field_dict["job_title"] = job_title
        if location is not UNSET:
            field_dict["location"] = location
        if location_id is not UNSET:
            field_dict["location_id"] = location_id
        if language is not UNSET:
            field_dict["language"] = language
        if timezone is not UNSET:
            field_dict["timezone"] = timezone
        if account_status is not UNSET:
            field_dict["account_status"] = account_status
        if emergency_contact_name is not UNSET:
            field_dict["emergency_contact_name"] = emergency_contact_name
        if emergency_contact_phone is not UNSET:
            field_dict["emergency_contact_phone"] = emergency_contact_phone
        if media_preferences is not UNSET:
            field_dict["media_preferences"] = media_preferences

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.media_preferences import MediaPreferences # noqa: PLC0415
        d = dict(src_dict)
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


        def _parse_enabled(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        enabled = _parse_enabled(d.pop("enabled", UNSET))


        def _parse_role(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        role = _parse_role(d.pop("role", UNSET))


        def _parse_department_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        department_id = _parse_department_id(d.pop("department_id", UNSET))


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


        def _parse_mobile_number(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        mobile_number = _parse_mobile_number(d.pop("mobile_number", UNSET))


        def _parse_pin(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        pin = _parse_pin(d.pop("pin", UNSET))


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


        def _parse_location_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        location_id = _parse_location_id(d.pop("location_id", UNSET))


        def _parse_language(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        language = _parse_language(d.pop("language", UNSET))


        def _parse_timezone(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        timezone = _parse_timezone(d.pop("timezone", UNSET))


        def _parse_account_status(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        account_status = _parse_account_status(d.pop("account_status", UNSET))


        def _parse_emergency_contact_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        emergency_contact_name = _parse_emergency_contact_name(d.pop("emergency_contact_name", UNSET))


        def _parse_emergency_contact_phone(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        emergency_contact_phone = _parse_emergency_contact_phone(d.pop("emergency_contact_phone", UNSET))


        def _parse_media_preferences(data: object) -> MediaPreferences | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                media_preferences_type_0 = MediaPreferences.from_dict(data)



                return media_preferences_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(MediaPreferences | None | Unset, data)

        media_preferences = _parse_media_preferences(d.pop("media_preferences", UNSET))


        update_user_request = cls(
            username=username,
            display_name=display_name,
            enabled=enabled,
            role=role,
            department_id=department_id,
            email=email,
            phone_number=phone_number,
            mobile_number=mobile_number,
            pin=pin,
            extension=extension,
            department=department,
            job_title=job_title,
            location=location,
            location_id=location_id,
            language=language,
            timezone=timezone,
            account_status=account_status,
            emergency_contact_name=emergency_contact_name,
            emergency_contact_phone=emergency_contact_phone,
            media_preferences=media_preferences,
        )


        update_user_request.additional_properties = d
        return update_user_request

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
