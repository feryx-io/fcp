from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="PageUserResponseDataItem")



@_attrs_define
class PageUserResponseDataItem:
    """ User response (excludes password hash).

        Attributes:
            id (str):
            username (str):
            tenant_id (str):
            enabled (bool):
            role (str):
            created_at (datetime.datetime):
            updated_at (datetime.datetime):
            language (str):
            timezone (str):
            account_status (str):
            failed_login_attempts (int):
            display_name (None | str | Unset):
            department_id (None | str | Unset):
            email (None | str | Unset):
            password_last_reset (datetime.datetime | None | Unset):
            phone_number (None | str | Unset):
            mobile_number (None | str | Unset):
            extension (None | str | Unset):
            department (None | str | Unset):
            job_title (None | str | Unset):
            location (None | str | Unset):
            location_id (None | str | Unset):
            last_login (datetime.datetime | None | Unset):
            account_locked_until (datetime.datetime | None | Unset):
            emergency_contact_name (None | str | Unset):
            emergency_contact_phone (None | str | Unset):
     """

    id: str
    username: str
    tenant_id: str
    enabled: bool
    role: str
    created_at: datetime.datetime
    updated_at: datetime.datetime
    language: str
    timezone: str
    account_status: str
    failed_login_attempts: int
    display_name: None | str | Unset = UNSET
    department_id: None | str | Unset = UNSET
    email: None | str | Unset = UNSET
    password_last_reset: datetime.datetime | None | Unset = UNSET
    phone_number: None | str | Unset = UNSET
    mobile_number: None | str | Unset = UNSET
    extension: None | str | Unset = UNSET
    department: None | str | Unset = UNSET
    job_title: None | str | Unset = UNSET
    location: None | str | Unset = UNSET
    location_id: None | str | Unset = UNSET
    last_login: datetime.datetime | None | Unset = UNSET
    account_locked_until: datetime.datetime | None | Unset = UNSET
    emergency_contact_name: None | str | Unset = UNSET
    emergency_contact_phone: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        id = self.id

        username = self.username

        tenant_id = self.tenant_id

        enabled = self.enabled

        role = self.role

        created_at = self.created_at.isoformat()

        updated_at = self.updated_at.isoformat()

        language = self.language

        timezone = self.timezone

        account_status = self.account_status

        failed_login_attempts = self.failed_login_attempts

        display_name: None | str | Unset
        if isinstance(self.display_name, Unset):
            display_name = UNSET
        else:
            display_name = self.display_name

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

        password_last_reset: None | str | Unset
        if isinstance(self.password_last_reset, Unset):
            password_last_reset = UNSET
        elif isinstance(self.password_last_reset, datetime.datetime):
            password_last_reset = self.password_last_reset.isoformat()
        else:
            password_last_reset = self.password_last_reset

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

        last_login: None | str | Unset
        if isinstance(self.last_login, Unset):
            last_login = UNSET
        elif isinstance(self.last_login, datetime.datetime):
            last_login = self.last_login.isoformat()
        else:
            last_login = self.last_login

        account_locked_until: None | str | Unset
        if isinstance(self.account_locked_until, Unset):
            account_locked_until = UNSET
        elif isinstance(self.account_locked_until, datetime.datetime):
            account_locked_until = self.account_locked_until.isoformat()
        else:
            account_locked_until = self.account_locked_until

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


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "username": username,
            "tenant_id": tenant_id,
            "enabled": enabled,
            "role": role,
            "created_at": created_at,
            "updated_at": updated_at,
            "language": language,
            "timezone": timezone,
            "account_status": account_status,
            "failed_login_attempts": failed_login_attempts,
        })
        if display_name is not UNSET:
            field_dict["display_name"] = display_name
        if department_id is not UNSET:
            field_dict["department_id"] = department_id
        if email is not UNSET:
            field_dict["email"] = email
        if password_last_reset is not UNSET:
            field_dict["password_last_reset"] = password_last_reset
        if phone_number is not UNSET:
            field_dict["phone_number"] = phone_number
        if mobile_number is not UNSET:
            field_dict["mobile_number"] = mobile_number
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
        if last_login is not UNSET:
            field_dict["last_login"] = last_login
        if account_locked_until is not UNSET:
            field_dict["account_locked_until"] = account_locked_until
        if emergency_contact_name is not UNSET:
            field_dict["emergency_contact_name"] = emergency_contact_name
        if emergency_contact_phone is not UNSET:
            field_dict["emergency_contact_phone"] = emergency_contact_phone

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        id = d.pop("id")

        username = d.pop("username")

        tenant_id = d.pop("tenant_id")

        enabled = d.pop("enabled")

        role = d.pop("role")

        created_at = datetime.datetime.fromisoformat(d.pop("created_at"))




        updated_at = datetime.datetime.fromisoformat(d.pop("updated_at"))




        language = d.pop("language")

        timezone = d.pop("timezone")

        account_status = d.pop("account_status")

        failed_login_attempts = d.pop("failed_login_attempts")

        def _parse_display_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        display_name = _parse_display_name(d.pop("display_name", UNSET))


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


        def _parse_password_last_reset(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                password_last_reset_type_0 = datetime.datetime.fromisoformat(data)



                return password_last_reset_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        password_last_reset = _parse_password_last_reset(d.pop("password_last_reset", UNSET))


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


        def _parse_last_login(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                last_login_type_0 = datetime.datetime.fromisoformat(data)



                return last_login_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        last_login = _parse_last_login(d.pop("last_login", UNSET))


        def _parse_account_locked_until(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                account_locked_until_type_0 = datetime.datetime.fromisoformat(data)



                return account_locked_until_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        account_locked_until = _parse_account_locked_until(d.pop("account_locked_until", UNSET))


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


        page_user_response_data_item = cls(
            id=id,
            username=username,
            tenant_id=tenant_id,
            enabled=enabled,
            role=role,
            created_at=created_at,
            updated_at=updated_at,
            language=language,
            timezone=timezone,
            account_status=account_status,
            failed_login_attempts=failed_login_attempts,
            display_name=display_name,
            department_id=department_id,
            email=email,
            password_last_reset=password_last_reset,
            phone_number=phone_number,
            mobile_number=mobile_number,
            extension=extension,
            department=department,
            job_title=job_title,
            location=location,
            location_id=location_id,
            last_login=last_login,
            account_locked_until=account_locked_until,
            emergency_contact_name=emergency_contact_name,
            emergency_contact_phone=emergency_contact_phone,
        )


        page_user_response_data_item.additional_properties = d
        return page_user_response_data_item

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
